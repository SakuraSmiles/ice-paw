// ChatMessages.process-narrative.test.ts — 组级过程叙述收纳回归锁
// （2026-09-16 拍板：多轮工具回合的过程碎句收进「过程叙述 · N 段」折叠行，
//   正文中区只留末段正文；展开=过程段回 item 原位，时间线与工具行交错复原。
//   同日三轮演进：二轮末段收束门控已**退役**（dev 实测碎句组全段直显依旧
//   「偏多偏杂」+ 生产库实证截断决定形态——异常终态大组 95% 冒号收尾，猜形态
//   = 猜不准）；三轮换轴 = 默认收纳全部 ≥3 段组 + 组后用户以「继续」续跑的
//   截断组换「回合被截断 · N 段过程」warning 标注（碎句尾段被语境化，不再
//   冒充结论）。空壳治理保留——收纳后无可见内容的 item 整个不渲染（旧形态
//   空 .message-item×12px 轮距堆出 380px+ 空白）。）
//
// 锁死十点：≥3 段默认折叠（只末段在场，折叠行恰一条）/ 展开回原位+收起态 /
// 2 段不收纳（轻量实质正文不动）/ 生成中不收纳（frozen-round）/ 末段=最后
// 有 content 的 item（末 item 纯工具轮不误判）/ 三区共存 DOM 序（胶囊行内
// 思考→工具→过程 → 末段正文，2026-09-16 四轮三行合一）/ 碎句组也默认收纳
// （门控退役）无后续不标截断 /
// 截断组「继续」续跑签名 → warning 标注 / 折叠态空壳 item 不渲染
// （.message-item 计数）/ 豁免卡 item 在场。
// 机制审计补锁（同日四轮）：分页前插并组键易主时——原全可见组预置展开 /
// 手动展开态随组转移（过程+工具两层同路）/ 旧组已收纳未展开不误预置 /
// @引用跳进收纳组自动展开（被引段直显）。
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

  it("≥3 段且末段完整收尾：默认折叠，折叠行恰一条报「过程叙述 · 3 段」，正文只末段在场", async () => {
    const w = await mountWith([
      textMsg("a1", "第一步说明："),
      textMsg("a2", "第二步说明："),
      textMsg("a3", "第三步说明："),
      textMsg("a4", "最终结论在这里。"),
    ]);

    const rows = w.findAll(".process-group-summary");
    expect(rows.length).toBe(1); // 组级恰一条（item v-for 之外的组级错位回归锁）
    expect(rows[0].text()).toContain("过程叙述 · 3 段");
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["最终结论在这里。"]);
  });

  it("展开回原位：全段回 item 原位、行切「收起」态；再点收回只余末段", async () => {
    const w = await mountWith([
      textMsg("a1", "第一步说明："),
      textMsg("a2", "第二步说明："),
      textMsg("a3", "第三步说明："),
      textMsg("a4", "最终结论在这里。"),
    ]);

    await w.findAll(".process-group-summary")[0].trigger("click");
    expect(w.findAll(".process-group-summary")[0].text()).toContain("收起 · 3 段过程叙述");
    expect(w.findAll(".md").map((m) => m.text())).toEqual([
      "第一步说明：", "第二步说明：", "第三步说明：", "最终结论在这里。",
    ]);

    await w.findAll(".process-group-summary")[0].trigger("click");
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["最终结论在这里。"]);
  });

  it("2 段不收纳：说明+结论的轻量两段全在场、无折叠行", async () => {
    const w = await mountWith([textMsg("a1", "我先看看文件结构"), textMsg("a2", "结论")]);

    expect(w.findAll(".process-group-summary").length).toBe(0);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["我先看看文件结构", "结论"]);
  });

  it("生成中不收纳（frozen-round 语义）：流式正文实时在场，回合结束沉淀", async () => {
    const w = await mountWith([
      textMsg("a1", "过程一："),
      textMsg("a2", "过程二："),
      textMsg("a3", "末段。"),
    ], true, 0); // sending + 组与生成窗口相交

    expect(w.findAll(".process-group-summary").length).toBe(0);
    expect(w.findAll(".md").length).toBe(3);

    const chat = useChatStore();
    chat.sending = false; // chat:done 收尾（clearTurnAnchors 清锚点）
    chat.turnFirstIdx = null;
    await flushPromises();

    expect(w.findAll(".process-group-summary").length).toBe(1);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["末段。"]);
  });

  it("末段判定：末 item 纯工具轮（无正文）时，末段 = 最后有 content 的 item", async () => {
    const w = await mountWith([
      textMsg("a1", "过程一："),
      textMsg("a2", "过程二："),
      textMsg("a3", "真正结论。"),
      msg({
        id: "a4", role: "assistant", model: "glm-5.3", content: "",
        content_blocks: JSON.stringify([{ type: "tool_use", id: "tu-1", name: "read_file", input: "{}" }]),
      }),
    ]);

    const rows = w.findAll(".process-group-summary");
    expect(rows.length).toBe(1);
    expect(rows[0].text()).toContain("过程叙述 · 2 段"); // count=3 → 过程段 2
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["真正结论。"]);
  });

  it("三区共存 DOM 序：胶囊行内 思考→工具→过程 → 末段正文（≥2 思考 + ≥3 正文 + ≥8 工具）", async () => {
    const msgs: Message[] = [msg({ id: "u1", role: "user", content: "任务" })];
    for (let i = 0; i < 4; i++) {
      msgs.push(msg({
        id: `a-${i}`, role: "assistant", model: "glm-5.3",
        content: i < 3 ? `过程 ${i}：` : "最终结论。",
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
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["最终结论。"]); // 只末段在场

    // 三胶囊合一排在同一行容器（2026-09-16 四轮：三行合一置气泡最前端）
    const pillsRow = w.find(".group-summary-pills");
    expect(pillsRow.exists()).toBe(true);
    expect(pillsRow.findAll(".summary-pill").length).toBe(3);

    // 语义图标锁（用户拍板换掉状态图标）：Brain 思考 / Wrench 工具 /
    // MessageSquareText 过程——lucide 默认类名携带图标名
    const pillsHtml = pillsRow.html();
    expect(pillsHtml).toContain("lucide-brain");
    expect(pillsHtml).toContain("lucide-wrench");
    expect(pillsHtml).toContain("lucide-message-square-text");

    // DOM 序：胶囊行内 思考 → 工具 → 过程 → 末段正文（顶部到中部）
    const html = w.find(".assistant-body").html();
    const positions = [
      html.indexOf("think-group-summary"),
      html.indexOf("tool-group-summary"),
      html.indexOf("process-group-summary"),
      html.indexOf("最终结论"),
    ];
    expect(positions.every((p) => p >= 0)).toBe(true);
    for (let i = 0; i < positions.length - 1; i++) {
      expect(positions[i]).toBeLessThan(positions[i + 1]);
    }
  });

  // ===== 三轮换轴（2026-09-16）：门控退役 + 截断标注；空壳治理保留 =====

  it("碎句组也默认收纳（门控退役）：末段冒号收尾、无『继续』后续 → 普通收纳行、无截断标注", async () => {
    const w = await mountWith([
      textMsg("a1", "第一步说明："),
      textMsg("a2", "第二步说明："),
      textMsg("a3", "第三步说明："),
      textMsg("a4", "接下来执行："),
    ]);

    // 二轮门控（segConcluded 形态代理）已退役：dev 实测碎句组全段直显依旧
    // 「偏多偏杂」，且生产库实证截断决定形态——判定轴从「猜末段形态」换成
    // 「读回合结尾事实」（groupTruncatedAfter）。无后续消息 = 拿不到续跑
    // 签名 → 普通收纳行（碎句尾段仍直显在场）。
    const rows = w.findAll(".process-group-summary");
    expect(rows.length).toBe(1);
    expect(rows[0].text()).toContain("过程叙述 · 3 段");
    expect(w.findAll(".process-truncated").length).toBe(0);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["接下来执行："]);
  });

  it("截断组标注：组后首条真 user 消息是「继续」→ 收纳行「回合被截断」+ warning 类", async () => {
    const w = await mountWith([
      textMsg("a1", "第一步说明："),
      textMsg("a2", "第二步说明："),
      textMsg("a3", "第三步说明："),
      textMsg("a4", "接下来执行："), // 撞轮数上限被截断的典型碎句尾段
      msg({ id: "u2", role: "user", content: "继续" }), // 「继续」按钮 = sendMessage('继续')
      textMsg("a5", "续跑后的结论。"),
    ]);

    // 生产实案 60/62 截断续跑全走「继续」按钮——它是截断组的形态签名。
    // 标注把碎句尾段语境化（「回合被截断」非结论），展开仍一键回原位。
    const rows = w.findAll(".process-group-summary");
    expect(rows.length).toBe(1); // 第二组仅 1 段不收纳
    expect(rows[0].text()).toContain("回合被截断 · 3 段过程");
    expect(w.findAll(".process-truncated").length).toBe(1);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["接下来执行：", "续跑后的结论。"]);

    // 展开态 = 事实已还原到行内，标注退场（收起态标签回归）
    await rows[0].trigger("click");
    expect(rows[0].text()).toContain("收起 · 3 段过程叙述");
    expect(w.findAll(".process-truncated").length).toBe(0);
  });

  it("空壳治理：折叠态无可见内容的 item 不渲染（.message-item 只剩末段 item）", async () => {
    const w = await mountWith([
      textMsg("a1", "第一步说明："),
      textMsg("a2", "第二步说明："),
      textMsg("a3", "第三步说明："),
      textMsg("a4", "最终结论在这里。"),
    ]);

    // 旧形态：a1-a3 内容藏了但 .message-item 骨架仍在，每个空壳吃 12px 轮距
    // ——50 轮组堆出 380px+ 空白（真机实案）。滤除后 DOM 里只剩末段 item。
    expect(w.findAll(".message-item").length).toBe(1);

    // 展开过程收纳：空壳回场（内容回来了）
    await w.findAll(".process-group-summary")[0].trigger("click");
    expect(w.findAll(".message-item").length).toBe(4);
  });

  it("空壳治理：与工具折叠共存——纯工具轮空壳同样滤除，展开工具行后回场", async () => {
    // 3 段正文（末段收尾）+ 每段 3 次工具 = 9 次 ≥8 → 工具折叠同场
    const msgs: Message[] = [msg({ id: "u1", role: "user", content: "任务" })];
    const texts = ["过程一：", "过程二：", "结论。"];
    texts.forEach((text, i) => {
      msgs.push(msg({
        id: `a-${i}`, role: "assistant", model: "glm-5.3", content: text,
        content_blocks: JSON.stringify([
          { type: "tool_use", id: `tu-${i}a`, name: "read_file", input: '{"path":"a.md"}' },
          { type: "tool_use", id: `tu-${i}b`, name: "read_file", input: '{"path":"b.md"}' },
          { type: "tool_use", id: `tu-${i}c`, name: "read_file", input: '{"path":"c.md"}' },
        ]),
      }));
      msgs.push(msg({
        id: `tr-${i}`, role: "user", content: "",
        content_blocks: JSON.stringify([
          { type: "tool_result", tool_use_id: `tu-${i}a`, content: "ok", is_error: false },
          { type: "tool_result", tool_use_id: `tu-${i}b`, content: "ok", is_error: false },
          { type: "tool_result", tool_use_id: `tu-${i}c`, content: "ok", is_error: false },
        ]),
      }));
    });
    const w = await mountWith(msgs);

    expect(w.findAll(".process-group-summary").length).toBe(1);
    expect(w.findAll(".tool-group-summary").length).toBe(1);
    // 双折叠同场：a-0/a-1 内容藏 + 工具藏 → 空壳滤除；只有 a-2（末段）在场
    expect(w.findAll(".message-item").length).toBe(1);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["结论。"]);

    // 展开工具行：a-0/a-1 的工具区回场 → item 回场（正文仍收纳）
    await w.findAll(".tool-group-summary")[0].trigger("click");
    expect(w.findAll(".message-item").length).toBe(3);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["结论。"]);
  });

  it("空壳治理：豁免卡（委派）独占的 item 不滤除——折叠态卡片在场", async () => {
    // a1 过程段 + 8 通用工具（达折叠阈值）；a2 过程段；a3 末段；a4 仅一张委派卡
    const msgs: Message[] = [
      msg({ id: "u1", role: "user", content: "任务" }),
      msg({
        id: "a1", role: "assistant", model: "glm-5.3", content: "过程一：",
        content_blocks: JSON.stringify(
          Array.from({ length: 8 }, (_, i) => ({ type: "tool_use", id: `tu-${i}`, name: "read_file", input: '{"path":"a.md"}' })),
        ),
      }),
      msg({
        id: "tr1", role: "user", content: "",
        content_blocks: JSON.stringify(
          Array.from({ length: 8 }, (_, i) => ({ type: "tool_result", tool_use_id: `tu-${i}`, content: "ok", is_error: false })),
        ),
      }),
      msg({
        id: "a2", role: "assistant", model: "glm-5.3", content: "过程二：",
        content_blocks: "[]",
      }),
      msg({
        id: "a3", role: "assistant", model: "glm-5.3", content: "结论完成。",
        content_blocks: "[]",
      }),
      msg({
        id: "a4", role: "assistant", model: "glm-5.3", content: "",
        content_blocks: JSON.stringify([{ type: "tool_use", id: "tu-dl", name: "delegate_to_agent", input: '{"agent_id":"dev-2","task":"t"}' }]),
      }),
      msg({
        id: "tr2", role: "user", content: "",
        content_blocks: JSON.stringify([{ type: "tool_result", tool_use_id: "tu-dl", content: '{"child_conversation_id":"c9","agent_name":"dev-2","finish_reason":"stop","rounds":3}', is_error: false }]),
      }),
    ];
    const w = await mountWith(msgs);

    // a1/a2（过程段，a1 工具折叠）空壳滤除；a3 末段在场；a4 委派卡豁免不滤
    expect(w.findAll(".message-item").length).toBe(2);
    expect(w.findAll(".delegation-stub").length).toBe(1); // 折叠态豁免卡在场
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["结论完成。"]);
  });

  // ===== 机制审计修复（2026-09-16）：分页前插合并的收纳态保全 + 跳转展开 =====

  it("分页前插合并：原 2 段全可见组并入达 3 段 → 预置展开（阅读连续优先于默认收纳）", async () => {
    const chat = useChatStore();
    const w = await mountWith([
      textMsg("a1", "第一步说明："),
      textMsg("a2", "结论。"),
    ]);
    expect(w.findAll(".process-group-summary").length).toBe(0); // 2 段不收纳
    expect(w.findAll(".md").length).toBe(2); // 用户正看着全部两段

    // 模拟 loadMoreMessages 前插：更早的同组 assistant 并进窗口首组（组键易主 grp-a1 → grp-a0）。
    // 修复前：新组首达阈值 → 默认收纳收走正在读的内容（长回合跨多页每次翻页都命中）
    chat.messages = [textMsg("a0", "第零步说明："), ...chat.messages];
    await flushPromises();

    const rows = w.findAll(".process-group-summary");
    expect(rows.length).toBe(1);
    expect(rows[0].text()).toContain("收起 · 2 段过程叙述"); // 预置展开态（非默认收纳）
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["第零步说明：", "第一步说明：", "结论。"]);
  });

  it("分页前插合并：用户手动展开的收纳组键易主后保持展开（展开态随组转移）", async () => {
    const chat = useChatStore();
    const w = await mountWith([
      textMsg("a1", "过程一："),
      textMsg("a2", "过程二："),
      textMsg("a3", "结论。"),
    ]);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["结论。"]); // 默认收纳
    await w.findAll(".process-group-summary")[0].trigger("click"); // 手动展开
    expect(w.findAll(".md").length).toBe(3);

    chat.messages = [textMsg("a0", "过程零："), ...chat.messages];
    await flushPromises();

    // 修复前：旧键 grp-a1 从展开集消失 → 回落默认收纳（键易主 = 展开态静默重置）
    expect(w.findAll(".process-group-summary")[0].text()).toContain("收起 · 3 段过程叙述");
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["过程零：", "过程一：", "过程二：", "结论。"]);
  });

  it("分页前插合并：旧组已达阈值且未展开 → 维持默认收纳（不误预置）", async () => {
    const chat = useChatStore();
    const w = await mountWith([
      textMsg("a1", "过程一："),
      textMsg("a2", "过程二："),
      textMsg("a3", "结论。"),
    ]);
    expect(w.findAll(".process-group-summary")[0].text()).toContain("过程叙述 · 2 段"); // 从未展开

    chat.messages = [textMsg("a0", "过程零："), ...chat.messages];
    await flushPromises();

    // 用户没展开过 → 并组后仍是默认收纳（预置只救「原本全可见」的组）
    expect(w.findAll(".process-group-summary")[0].text()).toContain("过程叙述 · 3 段");
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["结论。"]);
  });

  it("分页前插合并：工具折叠展开态同路转移（组级三收纳共用转移通道）", async () => {
    const chat = useChatStore();
    const toolItem = (id: string): Message => msg({
      id, role: "assistant", model: "glm-5.3", content: "",
      content_blocks: JSON.stringify([
        { type: "tool_use", id: `tu-${id}a`, name: "read_file", input: '{"path":"a.md"}' },
        { type: "tool_use", id: `tu-${id}b`, name: "read_file", input: '{"path":"b.md"}' },
      ]),
    });
    const w = await mountWith([toolItem("a1"), toolItem("a2"), toolItem("a3"), toolItem("a4")]);
    expect(w.findAll(".tool-group-summary").length).toBe(1); // 8 次通用工具 ≥8 默认折叠
    await w.findAll(".tool-group-summary")[0].trigger("click"); // 手动展开
    expect(w.findAll(".tool-group-summary")[0].text()).toContain("收起 · 8 次工具调用");

    chat.messages = [toolItem("a0"), ...chat.messages];
    await flushPromises();

    // 展开态随组转移到新键 grp-a0（10 次工具），不回落默认折叠
    expect(w.findAll(".tool-group-summary")[0].text()).toContain("收起 · 10 次工具调用");
  });

  it("@引用跳转进默认收纳组：落点组过程收纳自动展开（被引段直显，非只见折叠行）", async () => {
    // jsdom 无元素滚动实现，stub 掉 scrollTo（跳转定位本身不是本用例的断言面）
    Element.prototype.scrollTo = vi.fn();
    const w = await mountWith([
      msg({ id: "u1", role: "user", content: "任务" }),
      textMsg("a1", "过程一："),
      textMsg("a2", "过程二："),
      textMsg("a3", "过程三："),
      textMsg("a4", "结论。"),
      msg({
        id: "u2", role: "user", content: "",
        content_blocks: JSON.stringify([
          { type: "reference", ref_kind: "message", target_id: "a1", display: "消息#0001" },
        ]),
      }),
    ]);
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["结论。"]); // 组默认收纳

    await w.find(".user-ref-card").trigger("click"); // @引用跳转 → jumpToTurn(组首)
    await flushPromises();

    // 修复前：落点只见「过程叙述 · 3 段」折叠行、被引内容仍藏着
    expect(w.findAll(".md").map((m) => m.text())).toEqual(["过程一：", "过程二：", "过程三：", "结论。"]);
    expect(w.findAll(".process-group-summary")[0].text()).toContain("收起");
  });
});
