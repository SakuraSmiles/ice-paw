// composables/useChannel.ts
// 频道 v1 前端状态层：频道事件视图（选举聚合卡 + 通知条）+ 在途插话落流刷新。
//
// 数据流：
// - 会话选中：trajectory.listEvents 尾部窗口一次拉取（未过滤全量事件，cap
//   FETCH_WINDOW）——频道事件是行为事实（不物化系统消息行），渲染层与消息按
//   created_at 交错；
// - 增量：session:event-appended 过滤三 kind 追加（与 useInbox / 轨迹 live 同一
//   零轮询总线，append 落库成功才广播）；
// - 插话落流：频道会话的 user_message 事件 → loadMessages 权威刷新。C8 在途
//   插话无 chat:start（不打扰在途回合）——事件是插话可见的唯一驱动；正常
//   首条消息亦经此回正（store 频道分支不做乐观 push，重拉无重复）。
//
// 选举聚合卡（2026-09-11 ②）：一届选举 started→vote×N→result 在聊天区聚成
// 一张卡（取代零散通知行——用户反馈「明明是选举一件事，零散信息重复出现」）。
// **只发生在渲染前**：数据层/轨迹/append-only 事件零改（用户的解读正是与事件
// 日志契合，聚合是纯视图层收益）。配套两件事：
// - 投票行（assistant_message 且 turn_id 前缀 election:）从消息分组跳过——
//   票面已进卡，气泡再显一遍即重复；窗口外旧选举无卡时 Set 不含 → 照常气泡
//   渲染，自然回退；
// - 与卡 result 配对的 channel_coordinator(elected) 通知抑制（防「当选」双显）。
//
// 生命周期：App.vue onMounted 调 initChannel()（与 initInbox 并列），返回幂等
// cleanup。会话切换拉取由 ChatMessages 的 activeConvId watcher 驱动
// loadChannelNotices（模块级单例状态，keep-alive 生命周期不经过组件）。

import { ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { bridge } from "../api/bridge";
import { useChatStore } from "../stores/chat";
import type {
  ChannelCoordinatorPayload,
  ChannelElectionPayload,
  ChannelElectionResult,
  ChannelElectionVote,
  SessionEvent,
} from "../types";

/** 频道行为事件三 kind（后端 event_log 词表；渲染/轨迹两处消费同源） */
const CHANNEL_EVENT_KINDS = new Set(["channel_election", "channel_coordinator", "channel_mention"]);

/** 选举 turn_id 前缀（后端 run_election；同届选举事件同 turn_id，投票行事件同前缀） */
const ELECTION_TURN_PREFIX = "election:";

/** 拉取窗口：尾部事件数上限（一届选举 started+vote×N+result 多条，窗口过小会截断） */
const FETCH_WINDOW = 200;
/** 通知条渲染上限：更早的频道事件不显示（完整事实流在轨迹页，聊天区只管近期语境）。
 *  选举卡不受此限（聚合后天然紧凑，窗口上限已兜底）。 */
const RENDER_CAP = 30;

/** 当前会话的频道通知条（seq 升序；仅频道会话有内容，切走即清；选举三 phase
 *  已被聚合卡吸收，不在此列） */
const notices = ref<SessionEvent[]>([]);
/** 当前会话的选举聚合卡（按 started 时间升序） */
const electionCards = ref<ElectionCard[]>([]);
/** 本窗口内选举投票行的 message_id 集（ChatMessages 据此跳过气泡渲染——票面进卡） */
const electionVoteIds = ref<Set<string>>(new Set());
/** notices / cards 归属会话（bus 增量据此过滤；与 loadChannelNotices 的目标对齐） */
let boundConvId: string | null = null;

/** 一届选举的聚合视图（started→vote×N→result 一张卡） */
export interface ElectionCard {
  /** turn_id（"election:{uuid}"）——同届事件的归组键 */
  key: string;
  /** started 事件时间（与消息交错的锚点） */
  createdAt: string;
  /** 按投票先后序（含弃权票） */
  votes: ChannelElectionVote[];
  /** null = 进行中 / 中断（无 result 事件） */
  result: ChannelElectionResult | null;
}

/** 组件侧入口：当前会话频道视图（只读） */
export function useChannel() {
  return { notices, electionCards, electionVoteIds };
}

/** 会话切换拉取（ChatMessages 的 activeConvId watcher 调用；非频道清空）。
 *  竞态守卫：await 期间再切会话——晚到的旧响应按目标会话 id 丢弃。 */
export async function loadChannelNotices(convId: string | null): Promise<void> {
  boundConvId = convId;
  const chat = useChatStore();
  const conv = convId ? chat.conversations.find((c) => c.id === convId) : null;
  if (!conv || conv.kind !== "channel") {
    notices.value = [];
    electionCards.value = [];
    electionVoteIds.value = new Set();
    return;
  }
  try {
    // listEvents(convId, N) = 尾部 N 条、seq 升序（list_tail reverse 语义）；
    // conv 在场 ⇒ convId 非空（find 的键就是它），窄化取 conv.id
    const evts = await bridge.trajectory.listEvents(conv.id, FETCH_WINDOW);
    if (boundConvId !== convId) return; // 拉取在途切走：丢弃过期响应
    applyEvents(evts);
  } catch (e) {
    console.error("加载频道事件失败:", e);
    notices.value = [];
    electionCards.value = [];
    electionVoteIds.value = new Set();
  }
}

/** 窗口事件 → 三个视图（通知条 / 选举卡 / 投票行 id 集）。
 *  evts 须 seq 升序（listEvents 语义）——started 先于 vote 先于 result 见。 */
function applyEvents(evts: SessionEvent[]): void {
  // ① 选举卡：channel_election 按 turn_id 归组
  const cardMap = new Map<string, ElectionCard>();
  for (const e of evts) {
    if (e.kind !== "channel_election" || !e.turn_id) continue;
    const p = e.payload as ChannelElectionPayload;
    let card = cardMap.get(e.turn_id);
    if (!card) {
      card = { key: e.turn_id, createdAt: e.created_at, votes: [], result: null };
      cardMap.set(e.turn_id, card);
    }
    if (p.phase === "vote" && p.vote) card.votes.push(p.vote);
    else if (p.phase === "result" && p.result) card.result = p.result;
    // started 只定锚（首见即建卡）；重复防御性忽略
  }
  const cards = [...cardMap.values()];

  // ② 投票行 id：选举 turn 的 assistant_message（后端 cast_vote 物化的票面行）
  const voteIds = new Set<string>();
  for (const e of evts) {
    if (e.kind === "assistant_message" && e.turn_id?.startsWith(ELECTION_TURN_PREFIX) && e.message_id) {
      voteIds.add(e.message_id);
    }
  }

  // ③ 通知条：剔除选举三 phase（进卡）+ 与卡配对的 elected 统筹事件（防「当选」
  //    双显）。appointed/removed/failed-over 与无卡 elected（窗口边缘裁掉了对应
  //    选举）照常保留——抑制判据是「窗口内有卡宣告同一胜者」。
  const winnerIds = new Set(
    cards.filter((c) => c.result?.winner_agent_id).map((c) => c.result!.winner_agent_id!),
  );
  const ns = evts
    .filter((e) => {
      if (!CHANNEL_EVENT_KINDS.has(e.kind)) return false;
      if (e.kind === "channel_election") return false; // 已进卡
      if (e.kind === "channel_coordinator") {
        const p = e.payload as ChannelCoordinatorPayload;
        if (p.action === "elected" && p.agent_id && winnerIds.has(p.agent_id)) return false;
      }
      return true;
    })
    .slice(-RENDER_CAP);

  electionCards.value = cards;
  electionVoteIds.value = voteIds;
  notices.value = ns;
}

/** boot + 事件订阅（App.vue 调一次；返回幂等 cleanup） */
export async function initChannel(): Promise<() => void> {
  const unlisten = await listen<{ conversation_id: string; kind: string }>(
    "session:event-appended",
    (e) => {
      const { conversation_id: cid, kind } = e.payload;
      if (CHANNEL_EVENT_KINDS.has(kind) || kind === "assistant_message") {
        if (cid !== boundConvId) return; // 只维护当前观看的会话（轨迹页有全量）
        // bus note 无 payload——异步补拉尾部增量（append 已落库，list 必含）；
        // assistant_message 也走补拉：投票行 id 集依赖事件的 turn_id，逐条拼不出
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
    electionCards.value = [];
    electionVoteIds.value = new Set();
    boundConvId = null;
  };
}

/** 补拉当前会话尾部的频道事件（cap 内幂等替换——比逐条拼 payload 稳，
 *  bus note 只有 kind 无 payload，本地拼不出完整通知行/卡） */
async function pullLatestNotice(convId: string): Promise<void> {
  try {
    const evts = await bridge.trajectory.listEvents(convId, FETCH_WINDOW);
    if (boundConvId !== convId) return;
    applyEvents(evts);
  } catch {
    // 拉取失败保持现状（下次事件再试；轨迹页始终有全量）
  }
}
