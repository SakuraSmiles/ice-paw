// ChatMessages.channel.test.ts — 频道 v1 渲染锁定：
// ① sender 维度分组（不同成员 assistant 各自成组绝不合并 / 同成员连续合并）
// ② 成员身份头（sender_agent_name 快照优先 + agent store 兜底「已退出成员」+
//    当前统筹者 Shield 微标 = channelView 当下投影）
// ③ 生成中发出标注（user 在前置 assistant 生成窗口内 → gen-time-flag）
// ④ 频道事件通知按 created_at 与消息组交错（组前 preInterstitials + 尾部尾巴；
//    未拦截成员接力通知吸入被点名成员气泡身份行——「@来源」小标注，拦截/落空保留居中条）
// ⑤ 选举聚合卡：一届选举一张卡与消息交错；投票行气泡跳过（票面进卡）
// ⑨ 同秒平局挂组：通知事件与占位行落同一墙钟秒 → 挂到该成员自己的组（答前）
// ⑩ 群聊气泡布局：头像独立气泡外（channel-avatar）+ 昵称行在气泡外（body 之外）
//    + 气泡体 = .assistant-body（内容结构与 1v1 一致）
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { ref } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import ChatMessages from "../ChatMessages.vue";
import { useChatStore } from "../../../stores/chat";
import { useAgentStore } from "../../../stores/agent";
import { useChannel, type ElectionCard } from "../../../composables/useChannel";
import type { Message, SessionEvent } from "../../../types";

const push = vi.fn();

vi.mock("vue-router", () => ({
  useRouter: () => ({ push, currentRoute: { value: { name: "Home", fullPath: "/" } } }),
}));
vi.mock("../MarkdownRenderer.vue", () => ({
  default: { name: "MarkdownRenderer", props: ["content", "streaming"], template: "<div class='md'>{{ content }}</div>" },
}));
vi.mock("../TurnRail.vue", () => ({ default: { name: "TurnRail", template: "<div />" } }));
vi.mock("../../../composables/useScrollFollow", () => ({
  useScrollFollow: () => ({
    showScrollBtn: ref(false),
    autoFollow: ref(true),
    paginating: ref(false),
    scrollToBottom: vi.fn(),
    restoreForConversation: vi.fn(),
  }),
}));
vi.mock("../../../composables/useTurnRail", () => ({
  useTurnRail: () => ({ anchors: ref([]), loadAnchors: vi.fn() }),
}));
vi.mock("../../../composables/useActiveTurn", () => ({
  useActiveTurn: () => ({
    activeTurn: ref(null),
    turnOfMsg: ref(new Map()),
    refresh: vi.fn(),
    pin: vi.fn(),
    clearPin: vi.fn(),
  }),
  THRESHOLD_PX: 200,
}));
vi.mock("../../../composables/useThinkingTimer", () => ({
  useThinkingTimer: () => ({ thinkingElapsed: ref("") }),
}));

function msg(partial: Partial<Message> & { id: string; role: string }): Message {
  return {
    conversation_id: "ch1",
    content: "",
    content_blocks: "[]",
    token_count: null,
    error: null,
    created_at: "2026-09-10 10:00:00",
    rowid: 0,
    model: null,
    ...partial,
  } as Message;
}

let seq = 0;
function noticeEv(payload: unknown, createdAt: string): SessionEvent {
  seq += 1;
  return {
    id: seq, session_id: "ch1", seq, kind: "channel_mention", actor: "user",
    turn_id: null, message_id: null, payload: payload as never, created_at: createdAt,
  };
}

async function mountChannel(messages: Message[], coordinator: string | null = "ag1") {
  const chat = useChatStore();
  chat.conversations = [{
    id: "ch1", agent_id: "ag1", title: "项目频道", pinned: false,
    created_at: "2026-09-10 00:00:00", updated_at: "2026-09-10 00:00:00",
    project_id: "p1", kind: "channel", archived_at: null,
  }];
  chat.activeConvId = "ch1";
  chat.messages = messages;
  chat.channelView = {
    channel: null,
    members: [
      { agent_id: "ag1", name: "写手", role: "coordinator" },
      { agent_id: "ag2", name: "审校", role: "member" },
    ],
    coordinator_agent_id: coordinator,
    coordinator_appointed: false,
  };
  const w = mount(ChatMessages);
  await flushPromises();
  return w;
}

describe("ChatMessages 频道渲染", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockReset();
    const agents = useAgentStore();
    agents.list = [
      { id: "ag1", name: "写手", provider: "zhipu", model: "glm-5.3", system_prompt: "", base_url: null, temperature: 0.7, max_tokens: 4096, extra_params: {}, sort_order: 0, cache_prompt: true, has_api_key: true, created_at: "", updated_at: "", avatar: null },
      { id: "ag2", name: "审校", provider: "zhipu", model: "glm-5.3", system_prompt: "", base_url: null, temperature: 0.7, max_tokens: 4096, extra_params: {}, sort_order: 0, cache_prompt: true, has_api_key: true, created_at: "", updated_at: "", avatar: null },
    ] as never;
    // 模块级单例状态重置（上一个用例注入的 notices / 选举卡不泄漏）
    const ch = useChannel();
    ch.notices.value = [];
    ch.electionCards.value = [];
    ch.electionVoteIds.value = new Set();
  });

  it("sender 维度分组：同成员连续 assistant 合并，不同成员绝不合并", async () => {
    const w = await mountChannel([
      msg({ id: "u1", role: "user", content: "开始" }),
      msg({ id: "a1", role: "assistant", content: "写手第一段", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a2", role: "assistant", content: "写手第二段", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a3", role: "assistant", content: "审校接话", sender_agent_id: "ag2", sender_agent_name: "审校" }),
      msg({ id: "a4", role: "assistant", content: "无署名（旧消息未 enrich）", sender_agent_id: null }),
    ]);

    const groups = w.findAll(".message-group.assistant");
    expect(groups.length).toBe(3); // ag1 合并一组 + ag2 一组 + null 一组
    // 身份头三组各一：ag1 / 审校 / （无头）
    const heads = w.findAll(".channel-sender-head .channel-sender-name").map((n) => n.text());
    expect(heads).toEqual(["写手", "审校"]);
    // data-mid=组首消息 id（滚动锚点语义）：ag1 组锚 a1，组内两条内容都在
    expect(groups[0].attributes("data-mid")).toBe("a1");
    expect(groups[0].text()).toContain("写手第一段");
    expect(groups[0].text()).toContain("写手第二段");
    expect(groups[1].attributes("data-mid")).toBe("a3");
  });

  it("统筹者 Shield 微标 = channelView 当下投影（ag1 是统筹 → 唯一头带盾）", async () => {
    const w = await mountChannel([
      msg({ id: "a1", role: "assistant", content: "写手", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a2", role: "assistant", content: "审校", sender_agent_id: "ag2", sender_agent_name: "审校" }),
    ]);
    expect(w.findAll(".channel-sender-shield").length).toBe(1);
    expect(w.find(".channel-sender-head .channel-sender-shield").exists()).toBe(true);
  });

  it("名字链：sender_agent_name 快照优先；null 快照走 store；双查无 →「已退出成员」", async () => {
    const w = await mountChannel([
      // 快照「老写手」与 store 名（写手）不同——快照赢（改名不追溯）
      msg({ id: "a1", role: "assistant", content: "历史", sender_agent_id: "ag1", sender_agent_name: "老写手" }),
      // 快照空 + agent 已删（store 查无 ag-gone）
      msg({ id: "a2", role: "assistant", content: "幽灵", sender_agent_id: "ag-gone", sender_agent_name: null }),
    ]);
    const names = w.findAll(".channel-sender-head .channel-sender-name").map((n) => n.text());
    expect(names).toEqual(["老写手", "已退出成员"]);
  });

  it("生成中发出标注：user 落在前置 assistant 生成窗口内 → gen-time-flag；无 duration 不标注", async () => {
    const w = await mountChannel([
      // 生成窗口 10:00:00 + 30s = 10:00:30；插话 10:00:15 在窗口内
      msg({ id: "a1", role: "assistant", content: "正在生成", created_at: "2026-09-10 10:00:00", turn_duration_ms: 30000 }),
      msg({ id: "u1", role: "user", content: "你这个不对", created_at: "2026-09-10 10:00:15" }),
      // 旧消息无 duration（未 enrich）→ 诚实不标注
      msg({ id: "a2", role: "assistant", content: "旧回答", created_at: "2026-09-10 10:01:00", turn_duration_ms: null }),
      msg({ id: "u2", role: "user", content: "跟一句", created_at: "2026-09-10 10:01:10" }),
    ]);
    const flags = w.findAll(".gen-time-flag");
    expect(flags.length).toBe(1);
    expect(w.find('[data-mid="u1"]').find(".gen-time-flag").exists()).toBe(true);
    expect(w.find('[data-mid="u2"]').find(".gen-time-flag").exists()).toBe(false);
  });

  it("频道事件通知按 created_at 交错：成员接力吸入 + 拦截居中 + 尾部尾巴（在途实时）", async () => {
    const w = await mountChannel([
      msg({ id: "u1", role: "user", content: "开始", created_at: "2026-09-10 10:00:00" }),
      msg({ id: "a1", role: "assistant", content: "写手答", created_at: "2026-09-10 10:00:05", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a2", role: "assistant", content: "审校答", created_at: "2026-09-10 10:01:00", sender_agent_id: "ag2", sender_agent_name: "审校" }),
    ]);
    // mount 后 watcher 的 loadChannelNotices 已 await（flushPromises）——此时注入。
    // fixtures 对齐 useChannel 过滤后的真实形态：用户自起首跳派发（from=null
    // 且未拦截）已被过滤，此处用成员接力（→吸入审校气泡身份行）+ 广播拦截（→居中条）
    useChannel().notices.value = [
      noticeEv({ from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 1, chain_remaining: 0, blocked_reason: null }, "2026-09-10T10:00:30Z"),
      noticeEv({ from_agent_id: null, to_agent_id: "ag1", broadcast: true, blocked_reason: "user_preempted" }, "2026-09-10T10:02:00Z"),
    ];
    await flushPromises();

    // 未拦截成员接力（写手→审校，10:00:30 在 ag2 组前）吸入审校组身份行：
    // 居中条退位为「@写手」小标注，完整句义进 hover title
    const src = w.find('[data-mid="a2"]').find(".channel-mention-src");
    expect(src.exists()).toBe(true);
    expect(src.text()).toBe("写手");
    expect(src.attributes("title")).toBe("写手 点名 审校 接力");
    // 拦截类（广播被用户插话取消，10:02:00）保留居中条，晚于所有组 → 尾部
    const notices = w.findAll(".channel-notice");
    expect(notices.length).toBe(1);
    expect(w.text()).toContain("广播 · 写手：用户插话，接力取消");
  });

  it("选举聚合卡：一届一张卡与消息交错；投票行气泡跳过（票面进卡不重复）", async () => {
    const w = await mountChannel([
      msg({ id: "u1", role: "user", content: "大家好", created_at: "2026-09-10 10:00:00" }),
      // 投票行（票面原文物化的 assistant 行；electionVoteIds 覆盖后不占气泡位）
      msg({ id: "vote-1", role: "assistant", content: "审校", created_at: "2026-09-10 10:00:02", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a1", role: "assistant", content: "当选感言", created_at: "2026-09-10 10:00:05", sender_agent_id: "ag2", sender_agent_name: "审校" }),
    ]);
    const card: ElectionCard = {
      key: "election:e1",
      createdAt: "2026-09-10T10:00:02Z",
      votes: [
        { voter_agent_id: "ag1", candidate_agent_id: "ag2", reason: null },
        { voter_agent_id: "ag2", candidate_agent_id: null, reason: "模型返回空（弃权）" },
      ],
      result: { tally: [], winner_agent_id: "ag2", tie_break: null },
    };
    const ch = useChannel();
    ch.electionCards.value = [card];
    ch.electionVoteIds.value = new Set(["vote-1"]);
    await flushPromises();

    // 卡渲染：标题 + 逐票行（投给/弃权+原因）+ 当选结果
    expect(w.findAll(".election-card").length).toBe(1);
    const cardText = w.find(".election-card").text();
    expect(cardText).toContain("统筹者选举");
    expect(cardText).toContain("投给 审校");
    expect(cardText).toContain("弃权");
    expect(cardText).toContain("审校 当选统筹者");
    // 投票行不占气泡位（data-mid 无 vote-1）；当选感言照常
    expect(w.find('[data-mid="vote-1"]').exists()).toBe(false);
    expect(w.find('[data-mid="a1"]').exists()).toBe(true);
    // 进行中卡（无 result）显示状态胶囊而非结果行
    ch.electionCards.value = [{ ...card, key: "election:e2", result: null }];
    await flushPromises();
    expect(w.find(".election-head-status").text()).toBe("进行中");
    expect(w.find(".election-result").exists()).toBe(false);
  });

  it("历史交错单元只挂一次：早于多个组的卡不随每条新消息重复；吸入通知尾部不复现（回归⑤）", async () => {
    const w = await mountChannel([
      msg({ id: "u1", role: "user", content: "第一条", created_at: "2026-09-10 10:00:00" }),
      msg({ id: "a1", role: "assistant", content: "写手答", created_at: "2026-09-10 10:00:05", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a2", role: "assistant", content: "审校答", created_at: "2026-09-10 10:01:00", sender_agent_id: "ag2", sender_agent_name: "审校" }),
    ]);
    // 卡早于所有组、接力通知（10:00:30）吸入 a2 组——各只出现一次。重复 bug
    // 形态：每组 filter 全量「time < 组开始」会把历史卡挂到后续每个组头上 +
    // TransitionGroup 重复 key 错位渲染；吸入漏收消费集则通知在尾部再出一条。
    const ch = useChannel();
    ch.electionCards.value = [{
      key: "election:e1",
      createdAt: "2026-09-10T09:59:00Z",
      votes: [],
      result: { tally: [], winner_agent_id: "ag1", tie_break: null },
    }];
    ch.notices.value = [
      noticeEv({ from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 1, chain_remaining: 0, blocked_reason: null }, "2026-09-10T10:00:30Z"),
    ];
    await flushPromises();

    expect(w.findAll(".election-card").length).toBe(1);
    expect(w.findAll(".channel-notice").length).toBe(0);
    expect(w.findAll(".channel-mention-src").length).toBe(1);
    // 位置：卡在最前组（u1）之前；吸入标注在 a2 组（被点名成员自己的气泡）
    const before = (a: { element: Element }, b: { element: Element }) =>
      (a.element.compareDocumentPosition(b.element) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
    expect(before(w.find(".election-card"), w.find('[data-mid="u1"]'))).toBe(true);
    expect(w.find('[data-mid="a2"]').find(".channel-mention-src").exists()).toBe(true);
    expect(w.find('[data-mid="a1"]').find(".channel-mention-src").exists()).toBe(false);
  });

  it("同秒平局挂组：派发通知与占位行同墙钟秒 → 吸入该成员自己的气泡（回归⑨）", async () => {
    const w = await mountChannel([
      msg({ id: "u1", role: "user", content: "开始", created_at: "2026-09-10 10:00:00" }),
      msg({ id: "a1", role: "assistant", content: "写手答", created_at: "2026-09-10 10:00:05", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a2", role: "assistant", content: "审校答", created_at: "2026-09-10 10:00:30", sender_agent_id: "ag2", sender_agent_name: "审校" }),
    ]);
    // 生产实案形态（DB 取证）：channel_mention 事件在派发时刻落库（chat_state.start
    // 后 spawn 前），审校占位行在 Pipeline 后几毫秒创建——两者落同一墙钟秒
    // 10:00:30。严格小于会把通知滑过 ag2 自己的组（掉尾部居中条、吸入标注
    // 消失）——时间感倒读。
    useChannel().notices.value = [
      noticeEv({ from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 1, chain_remaining: 0, blocked_reason: null }, "2026-09-10T10:00:30Z"),
    ];
    await flushPromises();

    // 吸入审校组身份行（@写手），无居中条、尾部零复现
    expect(w.findAll(".channel-notice").length).toBe(0);
    const src = w.find('[data-mid="a2"]').find(".channel-mention-src");
    expect(src.exists()).toBe(true);
    expect(src.text()).toBe("写手");
  });

  it("吸入锚定：仅未拦截成员接力且 to==组 sender 才吸入；拦截/落空保留居中条", async () => {
    const w = await mountChannel([
      msg({ id: "u1", role: "user", content: "开始", created_at: "2026-09-10 10:00:00" }),
      msg({ id: "a1", role: "assistant", content: "写手答", created_at: "2026-09-10 10:00:05", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a2", role: "assistant", content: "审校答", created_at: "2026-09-10 10:01:00", sender_agent_id: "ag2", sender_agent_name: "审校" }),
      msg({ id: "a3", role: "assistant", content: "无署名旧消息", created_at: "2026-09-10 10:02:00", sender_agent_id: null }),
    ]);
    useChannel().notices.value = [
      // 拦截类（chain_limit）——即便 to 命中组 sender 也保留居中条（护栏事实
      // 需要显性可见，吸入会把它藏进身份行）
      noticeEv({ from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 2, chain_remaining: 0, blocked_reason: "chain_limit" }, "2026-09-10T10:00:30Z"),
      // to 不命中任何组 sender（agX 已退出）→ 无组可吸，落下一组前居中条
      //（下一组恰是匿名组——user 组/匿名组同属「挂不上」兜底路径）
      noticeEv({ from_agent_id: "ag1", to_agent_id: "agX", hop_index: 1, chain_remaining: 0, blocked_reason: null }, "2026-09-10T10:01:30Z"),
    ];
    await flushPromises();

    expect(w.findAll(".channel-mention-src").length).toBe(0);
    const notices = w.findAll(".channel-notice");
    expect(notices.length).toBe(2);
    expect(notices[0].text()).toContain("写手 @ 审校：接力链达上限");
    expect(notices[1].text()).toContain("写手 点名 已退出成员 接力");
  });

  it("群聊气泡布局：头像/昵称在气泡外，气泡体=assistant-body；匿名组无头像无头（⑩）", async () => {
    const w = await mountChannel([
      msg({ id: "u1", role: "user", content: "开始" }),
      msg({ id: "a1", role: "assistant", content: "写手答", sender_agent_id: "ag1", sender_agent_name: "写手" }),
      msg({ id: "a2", role: "assistant", content: "审校答", sender_agent_id: "ag2", sender_agent_name: "审校" }),
      msg({ id: "a3", role: "assistant", content: "无署名旧消息", sender_agent_id: null }),
    ]);

    // 头像独立列：有 sender 的组各一枚（lg 档），匿名组零头像
    const avatars = w.findAll(".channel-avatar");
    expect(avatars.length).toBe(2);
    // 昵称行在气泡外：sender-head 不嵌在 assistant-body（气泡体）内
    expect(w.find(".assistant-body .channel-sender-head").exists()).toBe(false);
    expect(w.find(".assistant-wrap.channel .channel-sender-head").exists()).toBe(true);
    // 气泡体承接内容：消息 item 与 footer 都在 body 内；wrap 层级 body 外有头像+昵称行
    expect(w.find(".assistant-body .message-item").exists()).toBe(true);
    expect(w.find(".assistant-body .message-footer").exists()).toBe(true);
    // 匿名组：无头像无头，但仍有 body（grid 显式定位防塌进头像列）
    const groups = w.findAll(".message-group.assistant.channel-msg");
    expect(groups.length).toBe(3);
    expect(groups[2].find(".channel-avatar").exists()).toBe(false);
    expect(groups[2].find(".channel-sender-head").exists()).toBe(false);
    expect(groups[2].find(".assistant-body").exists()).toBe(true);
  });
});
