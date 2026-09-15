// ChatMessages.tool-collapse.test.ts — ③ 组级工具折叠回归锁定
// （生产实案 2026-09-15：23% 回合 >20 次工具、峰值 150 行刷屏）。
//
// 锁死八点：阈值下不折 / 阈值上默认折+展开收起 / 失败计数 warning /
// 豁免（委派·计划卡）不计数 / 折叠时豁免卡仍可见 / 生成中不折
// （frozen-round 修复语义——c9d2680）/ chat:done 后沉淀 / 组间隔离。
//
// 计数与渲染必须共用 structuredCardKind（R4）：「摘要说 N 实际显示 N±1」
// 是本机制最易出的错位，用例 4/5 从两个断言面锁。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { ref } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import ChatMessages from "../ChatMessages.vue";
import { useChatStore } from "../../../stores/chat";
import type { Message } from "../../../types";

const push = vi.fn();

vi.mock("vue-router", () => ({
  useRouter: () => ({ push, currentRoute: { value: { name: "Home", fullPath: "/" } } }),
}));

vi.mock("../MarkdownRenderer.vue", () => ({
  default: { name: "MarkdownRenderer", props: ["content", "streaming"], template: "<div class='md'>{{ content }}</div>" },
}));
vi.mock("../TurnRail.vue", () => ({ default: { name: "TurnRail", template: "<div />" } }));
// 豁免卡 mock 成透传 stub：本测试只断言在场性与计数，不涉卡片内部行为
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
    created_at: "2026-09-15 10:00:00",
    rowid: 0,
    model: null,
    ...partial,
  } as Message;
}

/** 通用工具块（read_file 有展示名摘要行） */
function tu(id: string) {
  return { type: "tool_use", id, name: "read_file", input: '{"path":"a.md"}' };
}

/** 一条 tool_result-only user 行（冻结分离形态：结果行不单独成气泡、不隔断组） */
function resultsRow(key: string, specs: { id: string; error?: boolean; content?: string }[]): Message {
  return msg({
    id: "tr-" + key,
    role: "user",
    content: "",
    content_blocks: JSON.stringify(
      specs.map((s) => ({
        type: "tool_result",
        tool_use_id: s.id,
        content: s.content ?? (s.error ? "文件不存在: a.md" : "ok"),
        is_error: !!s.error,
      })),
    ),
  });
}

/** 一个完整回合片段：user 提问 + assistant（n 条通用工具）+ 配对结果行 */
function turnWithTools(key: string, n: number, failIdx: number[] = []): Message[] {
  const ids = Array.from({ length: n }, (_, i) => `${key}-t${i}`);
  return [
    msg({ id: `u-${key}`, role: "user", content: `任务 ${key}` }),
    msg({
      id: `a-${key}`,
      role: "assistant",
      model: "glm-5.3",
      content: "",
      content_blocks: JSON.stringify(ids.map((id) => tu(id))),
    }),
    resultsRow(key, ids.map((id, i) => ({ id, error: failIdx.includes(i) }))),
  ];
}

/** 通用工具行（排除摘要行——摘要行也挂 .tool-toggle 类，是 toggle 载体） */
function plainToolRows(w: ReturnType<typeof mount>) {
  return w.findAll(".tool-toggle").filter((el) => !el.classes().includes("tool-group-summary"));
}

async function mountWith(messages: Message[], streaming = false, turnFirstIdx: number | null = null) {
  const chat = useChatStore();
  chat.activeConvId = "c1";
  chat.sending = streaming;
  chat.turnFirstIdx = turnFirstIdx;
  chat.messages = messages;
  chat.streamingText = "";
  chat.streamingThinking = "";
  chat.streamingToolCalls = new Map();
  const w = mount(ChatMessages);
  await flushPromises();
  return w;
}

describe("③ 组级工具折叠", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockReset();
  });

  it("阈值下（7 条）不折：全行可见、无摘要行", async () => {
    const w = await mountWith(turnWithTools("s", 7));
    expect(plainToolRows(w).length).toBe(7);
    expect(w.findAll(".tool-group-summary").length).toBe(0);
  });

  it("阈值上（10 条）默认折：只摘要行；点开见全行 + 收起态；再点收回", async () => {
    const w = await mountWith(turnWithTools("b", 10));
    // 默认折叠：通用行全隐、摘要行在场并报总量
    expect(plainToolRows(w).length).toBe(0);
    const summary = w.findAll(".tool-group-summary");
    expect(summary.length).toBe(1);
    expect(summary[0].text()).toContain("10 次工具调用");

    // 展开：全行回场，摘要切「收起」态
    await summary[0].trigger("click");
    expect(plainToolRows(w).length).toBe(10);
    expect(w.findAll(".tool-group-summary")[0].text()).toContain("收起 · 10 次工具调用");

    // 再点收回
    await w.findAll(".tool-group-summary")[0].trigger("click");
    expect(plainToolRows(w).length).toBe(0);
  });

  it("失败计数：含失败行组摘要带「N 失败」warning 徽记", async () => {
    const w = await mountWith(turnWithTools("f", 10, [2, 5]));
    const summary = w.findAll(".tool-group-summary")[0];
    expect(summary.text()).toContain("10 次工具调用");
    expect(summary.text()).toContain("2 失败");
    expect(summary.find(".tool-fail-count").exists()).toBe(true);
  });

  it("豁免不计数：8 通用 + 委派/计划卡 → 摘要恰 8（两口径一致）", async () => {
    const base = turnWithTools("e", 8);
    // 同一 assistant 组内补一张委派卡 + 一张计划卡（豁免谓词 = 渲染非通用行的工具）
    base[1] = msg({
      ...base[1],
      content_blocks: JSON.stringify([
        ...Array.from({ length: 8 }, (_, i) => tu(`e-t${i}`)),
        { type: "tool_use", id: "e-dl", name: "delegate_to_agent", input: '{"agent_id":"dev-2","task":"t"}' },
        { type: "tool_use", id: "e-pl", name: "update_plan", input: '{"steps":[{"text":"a","status":"pending"}]}' },
      ]),
    });
    base[2] = resultsRow("e2", [
      ...Array.from({ length: 8 }, (_, i) => ({ id: `e-t${i}` })),
      { id: "e-dl", content: '{"child_conversation_id":"c9","agent_name":"dev-2","finish_reason":"stop","rounds":3}' },
      { id: "e-pl", content: '{"ok":true}' },
    ]);
    const w = await mountWith(base);

    expect(plainToolRows(w).length).toBe(0); // 折叠态通用行全隐
    const summary = w.findAll(".tool-group-summary")[0];
    expect(summary.text()).toContain("8 次工具调用");
    expect(summary.text()).not.toContain("10 次工具调用");
  });

  it("折叠时豁免卡仍可见（委派/计划卡不随折叠隐藏）", async () => {
    const base = turnWithTools("v", 8);
    base[1] = msg({
      ...base[1],
      content_blocks: JSON.stringify([
        ...Array.from({ length: 8 }, (_, i) => tu(`v-t${i}`)),
        { type: "tool_use", id: "v-dl", name: "delegate_to_agent", input: '{"agent_id":"dev-2","task":"t"}' },
        { type: "tool_use", id: "v-pl", name: "update_plan", input: '{"steps":[{"text":"a","status":"pending"}]}' },
      ]),
    });
    base[2] = resultsRow("v2", [
      ...Array.from({ length: 8 }, (_, i) => ({ id: `v-t${i}` })),
      { id: "v-dl", content: '{"child_conversation_id":"c9","agent_name":"dev-2","finish_reason":"stop","rounds":3}' },
      { id: "v-pl", content: '{"ok":true}' },
    ]);
    const w = await mountWith(base);

    expect(w.findAll(".delegation-stub").length).toBe(1); // 委派卡在折叠态仍在场
    expect(w.findAll(".plan-stub").length).toBe(1); // 计划卡在折叠态仍在场
  });

  it("生成中不折（frozen-round 修复语义）：sending ∧ 本回合组 → 全行可见", async () => {
    // turnFirstIdx = 1（a-s 的 messages 下标）——组与生成窗口相交
    const w = await mountWith(turnWithTools("l", 10), true, 1);
    expect(plainToolRows(w).length).toBe(10);
    expect(w.findAll(".tool-group-summary").length).toBe(0);
  });

  it("chat:done 后沉淀：sending=false + 锚点清 → 折叠出现", async () => {
    const w = await mountWith(turnWithTools("d", 10), true, 1);
    expect(plainToolRows(w).length).toBe(10); // 生成中先全可见

    const chat = useChatStore();
    chat.sending = false; // chat:done 收尾（clearTurnAnchors 同步清锚点）
    chat.turnFirstIdx = null;
    await flushPromises();

    expect(plainToolRows(w).length).toBe(0); // 回合结束沉淀为折叠
    expect(w.findAll(".tool-group-summary").length).toBe(1);
  });

  it("组间隔离：两组各 10 条独立折叠，展开互不影响", async () => {
    const messages = [
      ...turnWithTools("g1", 10),
      // 普通 user 隔断（tool_result-only 行不隔断连续性，须真 user 消息）
      msg({ id: "u-mid", role: "user", content: "第二轮" }),
      ...turnWithTools("g2", 10),
    ];
    const w = await mountWith(messages);

    const summaries = w.findAll(".tool-group-summary");
    expect(summaries.length).toBe(2); // 两组各一条摘要行
    expect(plainToolRows(w).length).toBe(0); // 两组都默认折

    // 只展开第一组：第二组保持折叠
    await summaries[0].trigger("click");
    expect(plainToolRows(w).length).toBe(10); // 仅第一组展开
    expect(w.findAll(".tool-group-summary")[1].text()).toContain("10 次工具调用"); // 第二组仍是折叠态文案
  });

  it("多 item 轮次形态：摘要行恰一条/组（2026-09-15 真机实案回归锁）", async () => {
    // 生产实案：50 轮撞顶回合 = 50 条 assistant 消息（每轮一条、各 1 tool_use）
    // + 49 条 tool_result-only user 行 → 合并一组 total=50；组间由「继续」user
    // 消息隔开。摘要行曾落在 item v-for 内 → 50 条重复摘要行/组、用户气泡被
    // 挤出视野。单 item 多 tool_use 形态（上面各用例）测不出此错位——多 item
    // 形态必须独立锁定。
    const messages: Message[] = [];
    for (let t = 0; t < 3; t++) {
      messages.push(msg({ id: `u-cont-${t}`, role: "user", content: "继续" }));
      for (let r = 0; r < 10; r++) {
        messages.push(msg({
          id: `a-${t}-${r}`, role: "assistant", model: "glm-5.3", content: "",
          content_blocks: JSON.stringify([tu(`tu-${t}-${r}`)]),
        }));
        messages.push(msg({
          id: `tr-${t}-${r}`, role: "user", content: "",
          content_blocks: JSON.stringify([{
            type: "tool_result", tool_use_id: `tu-${t}-${r}`, content: "ok", is_error: false,
          }]),
        }));
      }
    }
    const w = await mountWith(messages);

    // 三组各恰一条摘要行（而非 10×3 条 item 级重复）
    const summaries = w.findAll(".tool-group-summary");
    expect(summaries.length).toBe(3);
    expect(summaries[0].text()).toContain("10 次工具调用");

    // 组间「继续」用户气泡全部在场（曾被重复摘要行挤出视野）
    const userBubbles = w.findAll(".message-group.user .message-bubble");
    expect(userBubbles.length).toBe(3);
    expect(userBubbles[0].text()).toContain("继续");
  });
});
