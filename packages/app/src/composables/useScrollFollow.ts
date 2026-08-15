// composables/useScrollFollow.ts
// 消息列表滚动跟随 + 分页加载 + **阅读位置记忆**（scroll restoration 根治）：
// - 流式生成时自动贴底，用户向上滚离底部则暂停跟随（边生成边看不被打扰）；
// - 靠近顶部触发分页并在加载后保持原滚动位置；
// - 每会话记「滚动锚点」：离开时在底部 → 回来贴底；在读历史 → 回到锚点
//   消息原视口位置（锚点在分页窗口外时先程序化翻页再定位）。
//   覆盖所有离开/回来路径：切会话（侧栏/任务胶囊/委派卡/面包屑）、
//   页面往返（设置↔聊天，KeepAlive）。切 tab（对话↔轨迹）由 ChatPage 的
//   visibility 叠放保 DOM 保滚动，不经此路径。
//
// composable 内部注册 onMounted（挂载滚动监听 + 初始贴底）/ onUnmounted（移除监听），
// 与父组件 KeepAlive 的 onActivated 并行触发。

import { ref, nextTick, onMounted, onUnmounted, type Ref } from "vue";
import { useChatStore } from "../stores/chat";

const FOLLOW_THRESHOLD = 120;

/** 每会话滚动锚点：视口顶那条消息（组首条）id + 距视口顶偏移 + 离开时是否在底部。 */
export interface ScrollAnchor {
  messageId: string;
  offset: number;
  atBottom: boolean;
}

/** 会话 id → 锚点。模块级：组件重挂载（HMR/未来多实例）也不丢阅读位置。
 *  条目极小（三个字段），随访问过的会话线性增长，无需淘汰。 */
const anchors = new Map<string, ScrollAnchor>();

/**
 * 恢复决策（纯函数，可测）：
 * - 无锚点 / 离开时在底部 → 贴底（跟随最新是聊天默认意图）
 * - 锚点消息在已加载窗口 → 原位恢复
 * - 不在窗口 → 先翻页加载（调用方循环 loadMoreMessages 后再定位）
 */
export function planScrollRestore(
  anchor: ScrollAnchor | undefined,
  loadedMessageIds: Set<string>,
): "bottom" | "restore" | "paginate" {
  if (!anchor || anchor.atBottom) return "bottom";
  return loadedMessageIds.has(anchor.messageId) ? "restore" : "paginate";
}

/** 二分找「视口顶部所压/其后第一条」消息组元素（[data-mid]，含 date-divider
 *  间的组）。content-visibility:auto 下屏外组是估算高度，但 offsetTop 仍单调
 *  递增（contain-intrinsic-size 保证正高度），二分成立且 O(log n) 免全量遍历。 */
function findTopGroupEl(el: HTMLElement): HTMLElement | null {
  const groups = el.querySelectorAll<HTMLElement>("[data-mid]");
  if (groups.length === 0) return null;
  let lo = 0;
  let hi = groups.length - 1;
  // 找最后一个 offsetTop <= scrollTop 的组；全都在视口顶下方时取第 0 个
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (groups[mid].offsetTop <= el.scrollTop) lo = mid;
    else hi = mid - 1;
  }
  return groups[lo];
}

export function useScrollFollow(listRef: Ref<HTMLElement | null>) {
  const chat = useChatStore();
  const showScrollBtn = ref(false);
  /** 是否「跟随底部」自动滚动。用户向上滚离底部时置 false，滚回底部或点「回到底部」恢复 true。*/
  const autoFollow = ref(true);
  /** 分页加载进行中（期间不触发自动跟随 / 不重复分页 / 不采样锚点） */
  const paginating = ref(false);
  let suppressScrollCheck = false;
  const scrollPosCache = { scrollHeight: 0, scrollTop: 0 };

  /** 按当前真实几何刷新按钮/跟随态。scroll 事件到不了的地方（内容塌陷/增高
   *  不触发 scroll、suppress 窗口内）状态会冻结在旧值——每次主动定位后调用。 */
  function refreshFollowState() {
    const el = listRef.value;
    if (!el) return;
    const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
    showScrollBtn.value = dist > 80;
    autoFollow.value = dist <= FOLLOW_THRESHOLD;
  }

  // ---- 锚点捕获：滚动停稳后 150ms 采样（快滑中间态无价值） ----
  let captureTimer: ReturnType<typeof setTimeout> | undefined;

  function captureAnchor() {
    const el = listRef.value;
    const cid = chat.activeConvId;
    // 过渡期不采样（真机首修竞态）：切会话瞬间 messages 清空 → 列表塌陷的
    // scroll 事件/迟到的采样会把跨会话脏位置写进新会话锚点 → 回来恢复到错位
    if (!el || !cid || chat.msgLoading || paginating.value || chat.messages.length === 0) return;
    const group = findTopGroupEl(el);
    if (!group) return;
    anchors.set(cid, {
      messageId: group.dataset.mid!,
      offset: group.offsetTop - el.scrollTop,
      atBottom: el.scrollHeight - el.scrollTop - el.clientHeight <= FOLLOW_THRESHOLD,
    });
  }

  function scheduleCapture() {
    if (captureTimer) clearTimeout(captureTimer);
    captureTimer = setTimeout(captureAnchor, 150);
  }

  // 检测滚动位置：非底部显示按钮，靠近顶部触发分页
  function onScroll() {
    if (suppressScrollCheck || paginating.value || chat.msgLoading) return;
    const el = listRef.value;
    if (!el) return;

    const distToBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    showScrollBtn.value = distToBottom > 80;
    // 用户向上滚离底部 → 停止跟随；滚回底部 → 恢复跟随
    autoFollow.value = distToBottom <= FOLLOW_THRESHOLD;

    scheduleCapture();

    // 分页触发：距顶部 200px 且还有更多数据
    if (el.scrollTop < 200 && chat.hasMore && !chat.loadingMore && !chat.sending) {
      paginating.value = true;
      scrollPosCache.scrollHeight = el.scrollHeight;
      scrollPosCache.scrollTop = el.scrollTop;
      chat.loadMoreMessages().then(() => {
        nextTick(() => {
          const newEl = listRef.value;
          if (newEl) {
            const added = newEl.scrollHeight - scrollPosCache.scrollHeight;
            newEl.scrollTop = scrollPosCache.scrollTop + added;
          }
          paginating.value = false;
          refreshFollowState();
        });
      });
    }
  }

  /** 定位到真实底部（content-visibility 屏外是估算高度，首帧后真实高度才
   *  稳定 → 单次 scrollTo 会差一截）。仅在仍处跟随态时校正，勿抢用户滚动。 */
  function snapToRealBottom() {
    const el = listRef.value;
    if (!el || !autoFollow.value) return;
    el.scrollTop = el.scrollHeight;
    refreshFollowState();
  }

  function scrollToBottom(smooth?: boolean) {
    const el = listRef.value;
    const cid = chat.activeConvId;
    if (!el) return;
    suppressScrollCheck = true;
    autoFollow.value = true; // 手动滚到底部 = 恢复跟随
    showScrollBtn.value = false;
    // 显式贴底同步改写锚点意图：suppress 窗口内滚动事件不采样，
    // 不写会残留「读历史」旧锚点 → 回来时错误原位恢复
    if (cid) anchors.set(cid, { messageId: "", offset: 0, atBottom: true });
    el.scrollTo({ top: el.scrollHeight, behavior: smooth !== false ? "smooth" : "instant" });
    if (smooth !== false) {
      // smooth 动画期间内容可能增高（图片/流式），一次 scrollHeight 快照会停在
      // 离底差一截处（「跳到最新不准确」的根因）——动画结束后按真高度校正一次
      setTimeout(() => {
        snapToRealBottom();
        suppressScrollCheck = false;
        refreshFollowState();
      }, 500);
    } else {
      setTimeout(() => {
        snapToRealBottom();
        suppressScrollCheck = false;
        refreshFollowState();
        // 首帧渲染后 content-visibility 真实高度才稳定，再校一档
        setTimeout(snapToRealBottom, 300);
      }, 50);
    }
  }

  /** 把当前会话定位到锚点消息原视口位置（消息须已渲染）。返回是否定位成功。 */
  async function positionAtAnchor(cid: string): Promise<boolean> {
    const a = anchors.get(cid);
    const el = listRef.value;
    if (!a || a.atBottom || !el) return false;
    const sel = `[data-mid="${CSS.escape(a.messageId)}"]`;
    const node = el.querySelector<HTMLElement>(sel);
    if (!node) return false;
    suppressScrollCheck = true;
    autoFollow.value = false; // 恢复到历史位 = 非跟随态（「跳到最新」按钮应显示）
    el.scrollTop = node.offsetTop - a.offset;
    await nextTick();
    // content-visibility 首屏外高度是估算值：锚点进视口后按真实 offsetTop 再校
    setTimeout(() => {
      const e2 = listRef.value;
      if (e2 && chat.activeConvId === cid) {
        const n2 = e2.querySelector<HTMLElement>(sel);
        if (n2) e2.scrollTop = n2.offsetTop - a.offset;
        refreshFollowState();
      }
      suppressScrollCheck = false;
    }, 150);
    return true;
  }

  /**
   * 会话切换/消息重载后的滚动恢复（msgLoading watcher 调用）：
   * 按锚点意图二分——贴底 or 锚点原位；锚点在分页窗口外则先翻页加载。
   * paginate/hasMore 经参数注入（store 细节不进 composable）。
   */
  async function restoreForConversation(
    cid: string,
    paginate: { loadMore: () => Promise<void>; hasMore: () => boolean },
  ): Promise<void> {
    // 恢复途中又切了会话 → 立即放弃（真机首修竞态：旧循环会往新会话里翻页）
    if (chat.activeConvId !== cid) return;
    const anchor = anchors.get(cid);
    const plan = planScrollRestore(anchor, new Set(chat.messages.map((m) => m.id)));
    if (plan === "bottom") {
      scrollToBottom(false);
      return;
    }
    if (plan === "restore") {
      await positionAtAnchor(cid);
      return;
    }
    // paginate：锚点在窗口外（用户翻过历史），翻页直到锚点入窗或加载尽
    paginating.value = true;
    try {
      while (
        chat.activeConvId === cid
        && paginate.hasMore()
        && !chat.messages.some((m) => m.id === anchor!.messageId)
      ) {
        await paginate.loadMore();
        await nextTick();
      }
      if (chat.activeConvId !== cid) return;
      // 消息被删/加载尽仍未命中 → 兜底贴底（宁贴底不悬空）
      if (!(await positionAtAnchor(cid))) scrollToBottom(false);
    } finally {
      paginating.value = false;
    }
  }

  onMounted(() => {
    listRef.value?.addEventListener("scroll", onScroll);
    scrollToBottom(false);
  });
  onUnmounted(() => {
    listRef.value?.removeEventListener("scroll", onScroll);
    if (captureTimer) clearTimeout(captureTimer);
  });

  return { showScrollBtn, autoFollow, paginating, scrollToBottom, restoreForConversation };
}
