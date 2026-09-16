// ChatMessages.thinking-aggregate.test.ts — 组级思考聚合回归锁
// （2026-09-16 拍板：组内 ≥2 段思考收进气泡顶部——总开关 + 展开顶部堆叠；
//   四轮起三收纳行合一为气泡最前端胶囊行：思考/工具/过程并排，思考胶囊
//   在首位，展开堆叠仍紧随胶囊行下方）。
//
// 锁死六点：≥2 段聚合（顶部一行 + item 内隐藏）/ 1 段不聚合（原位贴正文）/
// 展开堆叠（段块 + 段内交互）/ 生成中不聚合（frozen-round 语义——流式思考
// 实时在场）/ 与工具折叠共存（顶思底工同场）/ 聚合行组级恰一条（多 item
// 形态——工具摘要行组级错位同族回归）。
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

/** 一条带思考段的 assistant（正文走 content，思考走 blocks） */
function thinkMsg(id: string, thinking: string, durationMs: number | null, content = ""): Message {
  return msg({
    id, role: "assistant", model: "glm-5.3", content,
    content_blocks: JSON.stringify([
      { type: "thinking", thinking, ...(durationMs != null ? { duration_ms: durationMs } : {}) },
    ]),
  });
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

describe("组级思考聚合", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockReset();
  });

  it("≥2 段聚合：顶部一行总量（段数+总耗时），item 内思考行隐藏", async () => {
    const w = await mountWith([
      thinkMsg("a1", "先分析问题", 41000, "第一轮结论"),
      thinkMsg("a2", "再验证假设", 12000, "第二轮结论"),
    ]);

    const summary = w.findAll(".think-group-summary");
    expect(summary.length).toBe(1);
    expect(summary[0].text()).toContain("思考 · 2 段");
    expect(summary[0].text()).toContain("53s"); // 41000+12000 总耗时（formatThinkingMs 整秒形态）

    // item 内思考行隐藏（聚合区承载）；正文照常在场
    expect(w.findAll(".message-item .think-block").length).toBe(0);
    expect(w.findAll(".md").map((m) => m.text())).toContain("第一轮结论");
  });

  it("1 段不聚合：思考行原位贴正文（无聚合行）", async () => {
    const w = await mountWith([thinkMsg("a1", "仅有的一段思考", 5000, "结论")]);

    expect(w.findAll(".think-group-summary").length).toBe(0);
    expect(w.findAll(".message-item .think-block").length).toBe(1);
  });

  it("展开堆叠：点聚合行 → 顶部堆叠各段块，段可再点开看内容；再点收回", async () => {
    const w = await mountWith([
      thinkMsg("a1", "思考甲", 41000, "结论"),
      thinkMsg("a2", "思考乙", 12000),
    ]);

    await w.findAll(".think-group-summary")[0].trigger("click");
    const segs = w.findAll(".think-group-stack .think-block");
    expect(segs.length).toBe(2); // 两段堆叠在顶部区
    expect(segs[0].text()).toContain("41s");
    expect(w.findAll(".message-item .think-block").length).toBe(0); // item 内仍隐藏

    // 段内交互：点第一段 → 内容可见
    await segs[0].find(".think-toggle").trigger("click");
    expect(w.findAll(".think-group-stack .think-body .md")[0].text()).toBe("思考甲");

    // 收回聚合：堆叠区消失、item 内思考行仍隐藏（聚合生效恒隐藏）
    await w.findAll(".think-group-summary")[0].trigger("click");
    expect(w.findAll(".think-group-stack").length).toBe(0);
  });

  it("生成中不聚合（frozen-round 语义）：流式思考实时在原位", async () => {
    const w = await mountWith([
      thinkMsg("a1", "思考甲", 41000),
      thinkMsg("a2", "思考乙", 12000),
    ], true, 0); // sending + 组与生成窗口相交

    expect(w.findAll(".think-group-summary").length).toBe(0);
    // 原位可见 1 个（a1 已冻结轮）；末条 a2 是 live assistant——其思考走流式
    // 通道（chat.streamingThinking），无流式思考时不渲染，与聚合无关
    expect(w.findAll(".message-item .think-block").length).toBe(1);
  });

  it("与工具折叠共存：思考/工具同胶囊行并排（≥2 思考 + ≥8 工具组）", async () => {
    const msgs: Message[] = [msg({ id: "u1", role: "user", content: "任务" })];
    for (let i = 0; i < 8; i++) {
      msgs.push(msg({
        id: `a-${i}`, role: "assistant", model: "glm-5.3", content: "",
        content_blocks: JSON.stringify([
          { type: "thinking", thinking: `轮 ${i} 思考`, duration_ms: 3000 },
          { type: "tool_use", id: `tu-${i}`, name: "read_file", input: '{"path":"a.md"}' },
        ]),
      }));
      msgs.push(msg({
        id: `tr-${i}`, role: "user", content: "",
        content_blocks: JSON.stringify([{ type: "tool_result", tool_use_id: `tu-${i}`, content: "ok", is_error: false }]),
      }));
    }
    const w = await mountWith(msgs);

    expect(w.findAll(".think-group-summary").length).toBe(1);
    expect(w.findAll(".think-group-summary")[0].text()).toContain("8 段");
    expect(w.findAll(".tool-group-summary").length).toBe(1);
    expect(w.findAll(".tool-group-summary")[0].text()).toContain("8 次工具调用");
    // 两胶囊同排一行（2026-09-16 四轮：三行合一置气泡最前端）
    const pillsRow = w.find(".assistant-body .group-summary-pills");
    expect(pillsRow.find(".think-group-summary").exists()).toBe(true);
    expect(pillsRow.find(".tool-group-summary").exists()).toBe(true);
  });

  it("聚合行组级恰一条（多 item 形态——工具摘要行组级错位同族回归锁）", async () => {
    // 多轮组每轮一条消息各带 1 段思考 → 聚合行 1 条（而非 3 条 item 级重复）
    const w = await mountWith([
      thinkMsg("a1", "思考一", 10000, "结论一"),
      thinkMsg("a2", "思考二", 11000, "结论二"),
      thinkMsg("a3", "思考三", 12000, "结论三"),
    ]);

    expect(w.findAll(".think-group-summary").length).toBe(1);
    expect(w.findAll(".think-group-summary")[0].text()).toContain("3 段");
    expect(w.findAll(".message-item .think-block").length).toBe(0);
  });
});
