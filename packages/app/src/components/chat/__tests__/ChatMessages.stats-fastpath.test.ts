// ChatMessages.stats-fastpath.test.ts — Layer B 统计事实混源消费回归锁
// （回合制分页配套：list_messages_by_turns 读时算好 per-row stats 随页返回，
//   前端组级聚合 stats 优先、无 stats 回落本地解析）。
//
// 锁死四点：stats 快路径求和（零解析——空 blocks 也出计数）/ 混源组逐 item
// 分流（stats 行 + 解析行同组共存）/ 同 fixture 双算口径锁（stats 计数 ==
// 旧解析路径计数——豁免/计错口径两侧不漂移，与后端 message_cmd tests 互指）/
// 思考门槛短路（全 stats 且段数和 <2 免解析；≥2 照常解析聚合）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { ref } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import ChatMessages from "../ChatMessages.vue";
import { useChatStore } from "../../../stores/chat";
import type { Message, MessageFacts } from "../../../types";

const push = vi.fn();

vi.mock("vue-router", () => ({
  useRouter: () => ({ push, currentRoute: { value: { name: "Home", fullPath: "/" } } }),
}));

vi.mock("../MarkdownRenderer.vue", () => ({
  default: { name: "MarkdownRenderer", props: ["content", "streaming"], template: "<div class='md'>{{ content }}</div>" },
}));
vi.mock("../TurnRail.vue", () => ({ default: { name: "TurnRail", template: "<div />" } }));
vi.mock("../DelegationCard.vue", () => ({
  default: { name: "DelegationCard", props: ["agentName", "agentId", "task", "status", "childConvId", "finishReason", "rounds"], template: "<div class='delegation-stub' />" },
}));
vi.mock("../PlanCard.vue", () => ({
  default: { name: "PlanCard", props: ["items"], template: "<div class='plan-stub' />" },
}));

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
    conversation_id: "c1",
    content: "",
    content_blocks: "[]",
    token_count: null,
    error: null,
    created_at: "2026-09-16 10:00:00",
    rowid: 0,
    model: null,
    ...partial,
  } as Message;
}

function statsOf(tool_uses: number, tool_errors: number, think_segs: number): MessageFacts {
  return { tool_uses, tool_errors, think_segs };
}

/** 通用工具块（read_file） */
function tu(id: string) {
  return { type: "tool_use", id, name: "read_file", input: '{"path":"a.md"}' };
}

/** tool_result-only user 行（配对数据源，不单独成气泡） */
function resultsRow(key: string, specs: { id: string; error?: boolean }[]): Message {
  return msg({
    id: "tr-" + key,
    role: "user",
    content: "",
    content_blocks: JSON.stringify(
      specs.map((s) => ({
        type: "tool_result",
        tool_use_id: s.id,
        content: s.error ? "文件不存在: a.md" : "ok",
        is_error: !!s.error,
      })),
    ),
  });
}

async function mountWith(messages: Message[]) {
  const chat = useChatStore();
  chat.activeConvId = "c1";
  chat.sending = false;
  chat.turnFirstIdx = null;
  chat.messages = messages;
  chat.streamingText = "";
  chat.streamingThinking = "";
  chat.streamingToolCalls = new Map();
  const w = mount(ChatMessages);
  await flushPromises();
  return w;
}

describe("Layer B 统计事实混源消费", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockReset();
  });

  it("stats 快路径求和：content_blocks 为空也出计数（零解析证明——解析路径此处恒 0）", async () => {
    const w = await mountWith([
      msg({ id: "u1", role: "user", content: "任务" }),
      msg({
        id: "a1", role: "assistant", model: "glm-5.3",
        content_blocks: "[]", // 空 blocks：本地解析恒 0——计数只能来自 stats
        stats: statsOf(10, 2, 0),
      }),
    ]);

    const summary = w.findAll(".tool-group-summary");
    expect(summary.length).toBe(1);
    expect(summary[0].text()).toContain("10 次工具调用");
    expect(summary[0].text()).toContain("2 失败");
  });

  it("混源组逐 item 分流：stats 行 + 解析行同组共存，计数两头相加", async () => {
    // 形态：翻页加载的 DB 行带 stats + 在途冻结/live 行（无 stats）合进同组
    const w = await mountWith([
      msg({ id: "u1", role: "user", content: "任务" }),
      msg({
        id: "a1", role: "assistant", model: "glm-5.3",
        content_blocks: "[]",
        stats: statsOf(6, 1, 0), // stats 路径贡献 6/1
      }),
      msg({
        id: "a2", role: "assistant", model: "glm-5.3",
        content_blocks: JSON.stringify([tu("live-1"), tu("live-2")]), // 解析路径贡献 2/1
      }),
      resultsRow("live", [{ id: "live-1", error: true }, { id: "live-2" }]),
    ]);

    const summary = w.findAll(".tool-group-summary");
    expect(summary.length).toBe(1);
    expect(summary[0].text()).toContain("8 次工具调用"); // 6 + 2
    expect(summary[0].text()).toContain("2 失败"); // 1 + 1
  });

  it("同 fixture 双算口径锁：stats 计数 == 旧解析路径计数（豁免/计错两侧不漂移）", async () => {
    // fixture = ChatMessages.tool-collapse.test.ts「豁免不计数」同款 + 2 段思考：
    // 8 通用（2 失败）+ 委派卡（豁免）+ 计划卡（豁免）+ 2 思考段。
    // stats 值按后端 compute_page_facts 公式预计算（两侧口径锁的目标本身）。
    function build(withStats: boolean): Message[] {
      const blocks = [
        ...Array.from({ length: 8 }, (_, i) => tu(`e-t${i}`)),
        { type: "tool_use", id: "e-dl", name: "delegate_to_agent", input: '{"agent_id":"dev-2","task":"t"}' },
        { type: "tool_use", id: "e-pl", name: "update_plan", input: '{"steps":[{"text":"a","status":"pending"}]}' },
      ];
      return [
        msg({ id: "u-1", role: "user", content: "任务" }),
        msg({
          id: "a-1", role: "assistant", model: "glm-5.3",
          content_blocks: JSON.stringify([
            { type: "thinking", thinking: "第一段", duration_ms: 41000 },
            ...blocks,
            { type: "thinking", thinking: "第二段", duration_ms: 12000 },
          ]),
          stats: withStats ? statsOf(8, 2, 2) : undefined,
        }),
        resultsRow("e", [
          ...Array.from({ length: 8 }, (_, i) => ({ id: `e-t${i}`, error: i === 2 || i === 5 })),
          { id: "e-dl" },
          { id: "e-pl" },
        ]),
      ];
    }

    const parsed = await mountWith(build(false)); // 旧路径：纯本地解析
    const fast = await mountWith(build(true)); // 新路径：stats 求和

    const pTool = parsed.findAll(".tool-group-summary")[0].text();
    const fTool = fast.findAll(".tool-group-summary")[0].text();
    expect(fTool).toContain("8 次工具调用");
    expect(fTool).toContain("2 失败");
    expect(fTool).toBe(pTool); // 两口径逐字相等

    const pThink = parsed.findAll(".think-group-summary")[0].text();
    const fThink = fast.findAll(".think-group-summary")[0].text();
    expect(fThink).toContain("2 段");
    expect(fThink).toBe(pThink);
  });

  it("思考门槛短路：全 stats 且段数和 <2 免解析（不聚合，原位贴正文）", async () => {
    // 1 段思考 + stats.think_segs=1 → 门槛 2 不满足，与旧路径行为等价（不进聚合表）
    const w = await mountWith([
      msg({
        id: "a1", role: "assistant", model: "glm-5.3", content: "结论",
        content_blocks: JSON.stringify([{ type: "thinking", thinking: "仅有的一段" }]),
        stats: statsOf(0, 0, 1),
      }),
    ]);
    expect(w.findAll(".think-group-summary").length).toBe(0);
    expect(w.findAll(".message-item .think-block").length).toBe(1); // 原位在场
  });

  it("stats 在场不误伤聚合：段数和 ≥2 照常解析聚合（短路只在 <2 生效）", async () => {
    const w = await mountWith([
      msg({
        id: "a1", role: "assistant", model: "glm-5.3", content: "结论",
        content_blocks: JSON.stringify([
          { type: "thinking", thinking: "第一段", duration_ms: 41000 },
          { type: "thinking", thinking: "第二段", duration_ms: 12000 },
        ]),
        stats: statsOf(0, 0, 2),
      }),
    ]);
    expect(w.findAll(".think-group-summary").length).toBe(1);
    expect(w.findAll(".think-group-summary")[0].text()).toContain("2 段");
  });
});
