// ChatMessages.frozen-round.test.ts — 多轮工具回合「已冻结轮瞬态消失」回归锁定
// （生产实案 2026-09-03：agent 连续多轮调工具时，上一轮的工具记录在下一轮
//  TTFT / 纯文本流式期间整块消失，chat:done 后才恢复）。
//
// 根因：骨架隐藏门控（原 `!chat.sending`）是全局粒度，而「空占位不渲染内部
// 模板」语义只应作用于当前流式 item（isLiveAssistant）。纯工具轮冻结后
// content=""、全局 streamingThinking/toolCallList 属于下一轮，四项全 false →
// 已冻结轮的工具卡被 v-if 藏掉。
//
// 锁死三点：① 下一轮空占位期（TTFT）已冻结轮工具卡可见；② 下一轮纯文本
// 流式期（无 thinking 无 tool-call-start，正是生产消失窗口）仍可见；③ 下一轮
// 占位的 think-dots 骨架行为不回归（空占位自身不渲染内部模板）。
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

// 重渲染子组件收窄为透传（本测试不涉及其内部行为）
vi.mock("../MarkdownRenderer.vue", () => ({
  default: { name: "MarkdownRenderer", props: ["content", "streaming"], template: "<div class='md'>{{ content }}</div>" },
}));
vi.mock("../TurnRail.vue", () => ({ default: { name: "TurnRail", template: "<div />" } }));

// 滚动/锚点/计时 composables 与 bridge 解耦（本测试只关心渲染门控）
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
    created_at: "2026-09-03 10:00:00",
    rowid: 0,
    model: null,
    ...partial,
  } as Message;
}

/** 生产形态的多轮工具回合中间态：轮 1 = 已冻结纯工具轮（无文本无 thinking，
 *  工具卡只在 content_blocks 里）——旧门控四项全 false 的最小复现数据。 */
function frozenRoundMessages(turn2Content = ""): Message[] {
  return [
    msg({ id: "u1", role: "user", content: "帮我整理文档" }),
    msg({
      id: "a1",
      role: "assistant",
      model: "glm-5.3",
      content_blocks: JSON.stringify([
        { type: "tool_use", id: "t1", name: "read_file", input: '{"path":"a.md"}' },
      ]),
    }),
    // freezeCurrentAssistant 分离出的 tool_result-only user（分组跳过、不断连续性）
    msg({
      id: "tr1",
      role: "user",
      content_blocks: JSON.stringify([
        { type: "tool_result", tool_use_id: "t1", content: "文件内容", is_error: false },
      ]),
    }),
    // 轮 2：chat:assistant-start 刚 push 的占位（content 随下一轮流式填充）
    msg({ id: "a2", role: "assistant", model: "glm-5.3", content: turn2Content }),
  ];
}

async function mountStreaming(turn2Content = "") {
  const chat = useChatStore();
  chat.activeConvId = "c1";
  chat.sending = true;
  chat.messages = frozenRoundMessages(turn2Content);
  chat.streamingText = turn2Content;
  chat.streamingThinking = "";
  chat.streamingToolCalls = new Map();
  const w = mount(ChatMessages);
  await flushPromises();
  return w;
}

describe("多轮工具回合：已冻结轮不被骨架门控隐藏", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockReset();
  });

  it("下一轮空占位期（TTFT）已冻结纯工具轮的工具卡可见", async () => {
    const w = await mountStreaming();
    const toggles = w.findAll(".tool-toggle");
    expect(toggles.length).toBe(1);
    expect(toggles[0].text()).toContain("read_file");
  });

  it("下一轮纯文本流式期（无 thinking / tool-call-start）工具卡仍可见——生产消失窗口", async () => {
    const w = await mountStreaming("接下来我把第二章也整理进来");
    expect(w.findAll(".tool-toggle").length).toBe(1);
    expect(w.findAll(".tool-toggle")[0].text()).toContain("read_file");
    // 轮 2 的流式文本同屏在渲染（两轮内容并存，不是二选一）
    const bubbles = w.findAll(".message-bubble");
    expect(bubbles.some((b) => b.text().includes("第二章"))).toBe(true);
  });

  it("下一轮空占位自身的 think-dots 骨架不回归", async () => {
    const w = await mountStreaming();
    expect(w.find(".think-dots").exists()).toBe(true);
  });

  it("回合结束后（sending=false）工具卡照常可见——原有兜底行为", async () => {
    const w = await mountStreaming();
    const chat = useChatStore();
    chat.sending = false;
    await flushPromises();
    expect(w.findAll(".tool-toggle").length).toBe(1);
  });
});
