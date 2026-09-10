// composables/useChannel.ts
// 频道 v1 前端状态层：频道事件通知（选举/统筹位/点名路由）+ 在途插话落流刷新。
//
// 数据流：
// - 会话选中：trajectory.listEvents 尾部窗口一次拉取（filter 三 kind，cap 30）——
//   频道事件是行为事实（不物化系统消息行），渲染层与消息按 created_at 交错；
// - 增量：session:event-appended 过滤三 kind 追加（与 useInbox / 轨迹 live 同一
//   零轮询总线，append 落库成功才广播）；
// - 插话落流：频道会话的 user_message 事件 → loadMessages 权威刷新。C8 在途
//   插话无 chat:start（不打扰在途回合）——事件是插话可见的唯一驱动；正常
//   首条消息亦经此回正（store 频道分支不做乐观 push，重拉无重复）。
//
// 生命周期：App.vue onMounted 调 initChannel()（与 initInbox 并列），返回幂等
// cleanup。会话切换拉取由 ChatMessages 的 activeConvId watcher 驱动
// loadChannelNotices（模块级单例状态，keep-alive 生命周期不经过组件）。

import { ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { bridge } from "../api/bridge";
import { useChatStore } from "../stores/chat";
import type { SessionEvent } from "../types";

/** 频道行为事件三 kind（后端 event_log 词表；渲染/轨迹两处消费同源） */
const CHANNEL_EVENT_KINDS = new Set(["channel_election", "channel_coordinator", "channel_mention"]);

/** 拉取窗口：尾部事件数上限（一届选举 started+vote×N+result 多条，窗口过小会截断） */
const FETCH_WINDOW = 200;
/** 渲染上限：更早的频道事件不显示（完整事实流在轨迹页，聊天区只管近期语境） */
const RENDER_CAP = 30;

/** 当前会话的频道事件（seq 升序；仅频道会话有内容，切走即清） */
const notices = ref<SessionEvent[]>([]);
/** notices 归属会话（bus 增量据此过滤；与 loadChannelNotices 的目标对齐） */
let boundConvId: string | null = null;

/** 组件侧入口：当前会话频道事件（只读） */
export function useChannel() {
  return { notices };
}

/** 会话切换拉取（ChatMessages 的 activeConvId watcher 调用；非频道清空）。
 *  竞态守卫：await 期间再切会话——晚到的旧响应按目标会话 id 丢弃。 */
export async function loadChannelNotices(convId: string | null): Promise<void> {
  boundConvId = convId;
  const chat = useChatStore();
  const conv = convId ? chat.conversations.find((c) => c.id === convId) : null;
  if (!conv || conv.kind !== "channel") {
    notices.value = [];
    return;
  }
  try {
    // listEvents(convId, N) = 尾部 N 条、seq 升序（list_tail reverse 语义）；
    // conv 在场 ⇒ convId 非空（find 的键就是它），窄化取 conv.id
    const evts = await bridge.trajectory.listEvents(conv.id, FETCH_WINDOW);
    if (boundConvId !== convId) return; // 拉取在途切走：丢弃过期响应
    notices.value = evts.filter((e) => CHANNEL_EVENT_KINDS.has(e.kind)).slice(-RENDER_CAP);
  } catch (e) {
    console.error("加载频道事件失败:", e);
    notices.value = [];
  }
}

/** boot + 事件订阅（App.vue 调一次；返回幂等 cleanup） */
export async function initChannel(): Promise<() => void> {
  const unlisten = await listen<{ conversation_id: string; kind: string }>(
    "session:event-appended",
    (e) => {
      const { conversation_id: cid, kind } = e.payload;
      if (CHANNEL_EVENT_KINDS.has(kind)) {
        if (cid !== boundConvId) return; // 只维护当前观看的会话（轨迹页有全量）
        // bus note 无 payload——异步补拉尾部增量（append 已落库，list 必含）
        void pullLatestNotice(cid);
      } else if (kind === "user_message") {
        // 频道插话落流：仅当前观看的频道会话刷新（其它会话的消息由各自
        // useChatEvents 路径处理；非频道会话不走此分支）
        const chat = useChatStore();
        const conv = chat.conversations.find((c) => c.id === cid);
        if (conv?.kind === "channel" && cid === chat.activeConvId) {
          void chat.loadMessages(cid);
        }
      }
    },
  );
  return () => {
    unlisten();
    notices.value = [];
    boundConvId = null;
  };
}

/** 补拉当前会话尾部的频道事件（cap 内幂等替换——比逐条拼 payload 稳，
 *  bus note 只有 kind 无 payload，本地拼不出完整通知行） */
async function pullLatestNotice(convId: string): Promise<void> {
  try {
    const evts = await bridge.trajectory.listEvents(convId, FETCH_WINDOW);
    if (boundConvId !== convId) return;
    notices.value = evts.filter((e) => CHANNEL_EVENT_KINDS.has(e.kind)).slice(-RENDER_CAP);
  } catch {
    // 拉取失败保持现状（下次事件再试；轨迹页始终有全量）
  }
}
