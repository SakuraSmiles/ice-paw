// ChatMessages.process-narrative.test.ts — 组级过程叙述收纳回归锁
// （2026-09-16 拍板：多轮工具回合的过程碎句收进「过程叙述 · N 段」折叠行，
//   正文中区只留末段正文；展开=过程段回 item 原位，时间线与工具行交错复原）。
//
// 锁死六点：≥3 段默认折叠（只末段在场，折叠行恰一条）/ 展开回原位+收起态 /
// 2 段不收纳（说明+结论的轻量实质正文不动）/ 生成中不收纳（frozen-round）/
// 末段=最后有 content 的 item（末 item 纯工具轮不误判）/ 三区共存 DOM 序
// （顶思 → 过程行 → 末段正文 → 工具行）。
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

/** 一条纯正文 assistant（连续同 model 合并一组） */
function textMsg(id: string, content: string): Message {
  return msg({ id, role: "assistant", model: "glm-5.3", content, content_blocks: "[]" });
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

describe("组级过程叙述收纳", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockReset();
  });

  it("≥3 段默认折叠：折叠行恰一条报「过程叙述 · 3 段」，正文只末段在场", async () => {
    const w = await mountWith([
      textMsg("a1", "第一步说明："),
      textMsg("a2", "第二步说明："),
      textMsg("a3", "第三步说明："),
      textMsg("a4", "最终结论在这里"),
    ]);

    const rows = w.findAll(".process-group-summary");
    expect(rows.length).toBe(1); // 组级恰一条（item v-for 之外的组级错位回归锁）
    expect(rows[0].text()).toContain("过程叙述 · 3 段");
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["最终结论在这里"]);
  });

  it("展开回原位：全段回 item 原位、行切「收起」态；再点收回只余末段", async () => {
    const w = await mountWith([
      textMsg("a1", "第一步说明："),
      textMsg("a2", "第二步说明："),
      textMsg("a3", "第三步说明："),
      textMsg("a4", "最终结论在这里"),
    ]);

    await w.findAll(".process-group-summary")[0].trigger("click");
    expect(w.findAll(".process-group-summary")[0].text()).toContain("收起 · 3 段过程叙述");
    expect(w.findAll(".md").map((m) => m.text())).toEqual([
      "第一步说明：", "第二步说明：", "第三步说明：", "最终结论在这里",
    ]);

    await w.findAll(".process-group-summary")[0].trigger("click");
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["最终结论在这里"]);
  });

  it("2 段不收纳：说明+结论的轻量两段全在场、无折叠行", async () => {
    const w = await mountWith([textMsg("a1", "我先看看文件结构"), textMsg("a2", "结论")]);

    expect(w.findAll(".process-group-summary").length).toBe(0);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["我先看看文件结构", "结论"]);
  });

  it("生成中不收纳（frozen-round 语义）：流式正文实时在场，回合结束沉淀", async () => {
    const w = await mountWith([
      textMsg("a1", "过程一"),
      textMsg("a2", "过程二"),
      textMsg("a3", "末段"),
    ], true, 0); // sending + 组与生成窗口相交

    expect(w.findAll(".process-group-summary").length).toBe(0);
    expect(w.findAll(".md").length).toBe(3);

    const chat = useChatStore();
    chat.sending = false; // chat:done 收尾（clearTurnAnchors 清锚点）
    chat.turnFirstIdx = null;
    await flushPromises();

    expect(w.findAll(".process-group-summary").length).toBe(1);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["末段"]);
  });

  it("末段判定：末 item 纯工具轮（无正文）时，末段 = 最后有 content 的 item", async () => {
    const w = await mountWith([
      textMsg("a1", "过程一"),
      textMsg("a2", "过程二"),
      textMsg("a3", "真正结论"),
      msg({
        id: "a4", role: "assistant", model: "glm-5.3", content: "",
        content_blocks: JSON.stringify([{ type: "tool_use", id: "tu-1", name: "read_file", input: "{}" }]),
      }),
    ]);

    const rows = w.findAll(".process-group-summary");
    expect(rows.length).toBe(1);
    expect(rows[0].text()).toContain("过程叙述 · 2 段"); // count=3 → 过程段 2
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["真正结论"]);
  });

  it("三区共存 DOM 序：顶思 → 过程行 → 末段正文 → 工具行（≥2 思考 + ≥3 正文 + ≥8 工具）", async () => {
    const msgs: Message[] = [msg({ id: "u1", role: "user", content: "任务" })];
    for (let i = 0; i < 4; i++) {
      msgs.push(msg({
        id: `a-${i}`, role: "assistant", model: "glm-5.3",
        content: i < 3 ? `过程 ${i}` : "最终结论",
        content_blocks: JSON.stringify([
          { type: "thinking", thinking: `思考 ${i}`, duration_ms: 3000 },
          { type: "tool_use", id: `tu-${i}a`, name: "read_file", input: '{"path":"a.md"}' },
          { type: "tool_use", id: `tu-${i}b`, name: "read_file", input: '{"path":"b.md"}' },
        ]),
      }));
      msgs.push(msg({
        id: `tr-${i}`, role: "user", content: "",
        content_blocks: JSON.stringify([
          { type: "tool_result", tool_use_id: `tu-${i}a`, content: "ok", is_error: false },
          { type: "tool_result", tool_use_id: `tu-${i}b`, content: "ok", is_error: false },
        ]),
      }));
    }
    const w = await mountWith(msgs);

    expect(w.findAll(".think-group-summary").length).toBe(1); // 4 段思考聚合
    expect(w.findAll(".process-group-summary").length).toBe(1); // 4 段正文收纳
    expect(w.findAll(".tool-group-summary").length).toBe(1); // 8 次工具折叠
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["最终结论"]); // 只末段在场

    // DOM 序：思考行 < 过程行 < 末段正文 < 工具摘要行（顶部到中部到底部）
    const html = w.find(".assistant-body").html();
    const positions = [
      html.indexOf("think-group-summary"),
      html.indexOf("process-group-summary"),
      html.indexOf("最终结论"),
      html.indexOf("tool-group-summary"),
    ];
    expect(positions.every((p) => p >= 0)).toBe(true);
    for (let i = 0; i < positions.length - 1; i++) {
      expect(positions[i]).toBeLessThan(positions[i + 1]);
    }
  });
});
