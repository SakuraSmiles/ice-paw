// composables/useInbox.ts
// MA-3 跨会话收件箱的前端状态层：pending 来件计数（侧栏 badge / 聊天头入口）。
//
// 数据流：
// - boot：list_inbox_counts 一次批量拉全部会话计数（打开应用即恢复 badge）；
// - 增量：监听 session:event-appended（append 落库成功才广播，到达时行必可查）
//   过滤 cross_session_message / cross_session_message_settled 两 kind 维护
//   Map<convId, count>——与轨迹 live 追加同一通知源，零轮询；
// - hold 来件 OS 通知：仅失焦时（有焦时应用内 badge 可见，通知是重复打扰；
//   与审批通知同哲学）拉权威列表确认政策后纯提醒（不带 toast 按钮——按钮
//   协议是工具授权 oneshot，与收件箱处置不同域）。
//
// 计数是「气味」不是真相：settled 抵扣依赖本地算术，极端时序（bus 丢帧）可
// 漂移 ±1；popover 打开与处置后都走 list_inbox 权威刷新回正。
//
// 生命周期：App.vue onMounted 调 initInbox()（与 useChatEvents 并列），
// onUnmounted 调返回的 cleanup。组件侧只消费 useInbox() 的只读状态。

import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import { bridge } from "../api/bridge";
import { notifyApprovalNeeded } from "../utils/systemNotify";
import type { InboxItem } from "../types";

/** 全应用单例状态（模块级 reactive Map：组件外维护、组件内响应式消费） */
const pendingCounts = reactive(new Map<string, number>());

/** 便捷取值：无记录 = 0（count_pending_inbox_all 只含有来件的会话） */
function pendingOf(convId: string): number {
  return pendingCounts.get(convId) ?? 0;
}

/** 组件侧入口：只读计数状态（写路径全在 initInbox 的事件回调内） */
export function useInbox() {
  return { pendingCounts, pendingOf };
}

/** 已发过 OS 通知的来件 id（恰一次簿记；随权威列表收缩清理，防 Set 无界） */
const notifiedIds = new Set<string>();

/** hold 来件失焦通知：拉权威列表确认政策（通知只服务 hold——accept 自动
 *  消费无需批准动作，refuse 根本收不到）。fire-and-forget，失败静默。 */
async function notifyHeldArrival(convId: string): Promise<void> {
  try {
    const view = await bridge.inbox.list(convId);
    if (view.policy !== "hold") return;
    const live = new Set<string>();
    for (const item of view.items) {
      live.add(item.message_id);
      if (notifiedIds.has(item.message_id)) continue;
      notifiedIds.add(item.message_id);
      notifyHeldItem(item);
    }
    for (const id of notifiedIds) if (!live.has(id)) notifiedIds.delete(id);
  } catch {
    // 列表拉取失败：跳过本次通知（badge 仍在，下次来件再试）
  }
}

function notifyHeldItem(item: InboxItem): void {
  const preview = item.content.length > 60 ? `${item.content.slice(0, 60)}…` : item.content;
  // 不传 request_id = 纯提醒（toast 按钮是工具授权协议，收件箱处置须回应用内）
  void notifyApprovalNeeded(
    "IcePaw · 跨会话消息待批准",
    `来自「${item.source_conversation_title}」的 agent ${item.source_agent_name}：${preview}`,
  );
}

/** 权威刷新单会话计数（popover 打开 / 处置后调用，回正本地算术漂移） */
export async function refreshInboxCount(convId: string): Promise<void> {
  try {
    const view = await bridge.inbox.list(convId);
    if (view.items.length > 0) pendingCounts.set(convId, view.items.length);
    else pendingCounts.delete(convId);
  } catch {
    /* 刷新失败保持现状（badge 是气味，下次事件/打开再回正） */
  }
}

/** boot + 事件订阅（App.vue 调一次；返回幂等 cleanup） */
export async function initInbox(): Promise<() => void> {
  // boot：批量拉计数（失败静默——badge 缺席不阻塞应用，下次来件事件起增量可用）
  void bridge.inbox.counts().then((pairs) => {
    pendingCounts.clear();
    for (const [id, n] of pairs) pendingCounts.set(id, n);
  }).catch(() => {});

  const unlisten = await listen<{ conversation_id: string; kind: string }>(
    "session:event-appended",
    (e) => {
      const { conversation_id: cid, kind } = e.payload;
      if (kind === "cross_session_message") {
        pendingCounts.set(cid, (pendingCounts.get(cid) ?? 0) + 1);
        if (!document.hasFocus()) void notifyHeldArrival(cid);
      } else if (kind === "cross_session_message_settled") {
        const cur = pendingCounts.get(cid) ?? 0;
        if (cur <= 1) pendingCounts.delete(cid);
        else pendingCounts.set(cid, cur - 1);
      }
    },
  );

  return () => {
    unlisten();
    notifiedIds.clear();
  };
}
