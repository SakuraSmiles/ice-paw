// ChatMessages.show-image.test.ts — 汇报图 live 兜底归属回归锁
// （2026-10-29 他机生产实案：生成 show_image 效果图期间，live 汇报图经
//   showImagesFor 的 liveShowImages 全局兜底被渲染到**每一条** assistant 气泡里
//   ——长思考/多轮生成时整列表多处重复同一组图；done 落库后 streamingToolCalls
//   清空、历史渲染接管，才「回归正常」。根因 = 兜底未限定当前流式 item。）
//
// 锁死：live 兜底只属于当前流式 assistant（末条），其它 assistant item 历史
// 无图就是无图，不共享全局 liveShowImages。
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

function userMsg(id: string, content: string): Message {
  return msg({ id, role: "user", content });
}
function asstMsg(id: string, content: string): Message {
  return msg({ id, role: "assistant", model: "glm-5.3", content });
}

describe("汇报图 live 兜底归属", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockReset();
  });

  it("生成 show_image 期间：live 汇报图只出现在当前流式 assistant，不扩散到历史 assistant 气泡", async () => {
    const chat = useChatStore();
    chat.activeConvId = "c1";
    chat.messages = [
      userMsg("u1", "帮我出一张图"),
      asstMsg("a1", "收到，正在生成……"),
      userMsg("u2", "再出一张"),
    ];
    // 末条 = 当前流式 assistant（空占位，live 工具调用在场）
    chat.messages = [...chat.messages, asstMsg("a2", "")];
    const s = chat.ensureStream("c1");
    s.sending = true;
    s.streamingToolCalls = new Map([
      ["tc-img", {
        id: "tc-img",
        name: "show_image",
        arguments: "{}",
        ended: true,
        result: {
          content: "生成完成",
          isError: false,
          durationMs: 10,
          images: [{ data: "iVBORw0KGgo=", media_type: "image/png" }],
        },
      }],
    ]);

    const w = mount(ChatMessages);
    await flushPromises();

    // 修复前：liveShowImages 全局兜底 → 每条 assistant（a1、a2）都渲染一张
    // show-image-card（重复渲染）。修复后：仅流式 item a2 一张。
    const cards = w.findAll(".show-image-card");
    expect(cards.length).toBe(1);
  });

  it("历史 assistant 已落库 show_image 图时，仍走历史渲染不受 live 兜底干扰", async () => {
    const chat = useChatStore();
    chat.activeConvId = "c1";
    // a1 已落库：含 show_image tool_use，结果图在独立 tool_result user 行
    const showImageToolUse = JSON.stringify([
      { type: "tool_use", id: "tc-past", name: "show_image", input: "{}" },
    ]);
    const toolResultRow = JSON.stringify([
      { type: "tool_result", tool_use_id: "tc-past", content: "完成", is_error: false },
      { type: "image", data: "iVBORw0KGgo=", media_type: "image/png" },
    ]);
    chat.messages = [
      userMsg("u1", "出图"),
      msg({ id: "a1", role: "assistant", model: "glm-5.3", content: "", content_blocks: showImageToolUse }),
      msg({ id: "r1", role: "user", content: "", content_blocks: toolResultRow }),
    ];
    // 末条 = 当前流式 assistant，live 有另一张 show_image
    chat.messages = [...chat.messages, asstMsg("a2", "")];
    const s = chat.ensureStream("c1");
    s.sending = true;
    s.streamingToolCalls = new Map([
      ["tc-live", {
        id: "tc-live",
        name: "show_image",
        arguments: "{}",
        ended: true,
        result: {
          content: "完成",
          isError: false,
          durationMs: 10,
          images: [{ data: "iVBORw0KGgo=", media_type: "image/png" }],
        },
      }],
    ]);

    const w = mount(ChatMessages);
    await flushPromises();

    // a1（历史图）+ a2（live 图）各一张，共两张；无第三处重复。
    const cards = w.findAll(".show-image-card");
    expect(cards.length).toBe(2);
  });
});
