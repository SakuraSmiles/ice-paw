// composables/useScrollFollow.ts
// 消息列表滚动跟随 + 分页加载：流式生成时自动贴底，用户向上滚离底部则暂停跟随
// （边生成边看不被滚动条打扰），滚回底部或点「回到底部」恢复；靠近顶部触发分页
// 并在加载后恢复原滚动位置。从 ChatMessages.vue 抽出。
//
// composable 内部注册 onMounted（挂载滚动监听 + 初始贴底）/ onUnmounted（移除监听），
// 与父组件 KeepAlive 的 onActivated 并行触发（onActivated 里调 scrollToBottom 即可）。

import { ref, nextTick, onMounted, onUnmounted, type Ref } from "vue";
import { useChatStore } from "../stores/chat";

const FOLLOW_THRESHOLD = 120;

export function useScrollFollow(listRef: Ref<HTMLElement | null>) {
  const chat = useChatStore();
  const showScrollBtn = ref(false);
  /** 是否「跟随底部」自动滚动。用户向上滚离底部时置 false，滚回底部或点「回到底部」恢复 true。*/
  const autoFollow = ref(true);
  /** 分页加载进行中（期间不触发自动跟随 / 不重复分页） */
  const paginating = ref(false);
  let suppressScrollCheck = false;
  const scrollPosCache = { scrollHeight: 0, scrollTop: 0 };

  // 检测滚动位置：非底部显示按钮，靠近顶部触发分页
  function onScroll() {
    if (suppressScrollCheck || paginating.value) return;
    const el = listRef.value;
    if (!el) return;

    const distToBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    showScrollBtn.value = distToBottom > 80;
    // 用户向上滚离底部 → 停止跟随；滚回底部 → 恢复跟随
    autoFollow.value = distToBottom <= FOLLOW_THRESHOLD;

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
        });
      });
    }
  }

  function scrollToBottom(smooth?: boolean) {
    if (listRef.value) {
      suppressScrollCheck = true;
      autoFollow.value = true; // 手动滚到底部 = 恢复跟随
      showScrollBtn.value = false;
      listRef.value.scrollTo({ top: listRef.value.scrollHeight, behavior: smooth !== false ? "smooth" : "instant" });
      setTimeout(() => { suppressScrollCheck = false; }, smooth !== false ? 500 : 50);
    }
  }

  onMounted(() => {
    listRef.value?.addEventListener("scroll", onScroll);
    scrollToBottom(false);
  });
  onUnmounted(() => {
    listRef.value?.removeEventListener("scroll", onScroll);
  });

  return { showScrollBtn, autoFollow, paginating, scrollToBottom };
}
