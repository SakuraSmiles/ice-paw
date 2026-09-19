// useChatEvents.steer-silent.test.ts — Steer 插话打断的静默衔接锁定（设计稿 §11
// transition prompt deferred）：
// 后端 Steer 分支的打断与手动停止走同一 chat:done(abort) 通道（设计 §55 刻意
// 「与手动停止完全相同」），但呈现层必须分野——插话打断不该误显「已手动停止」。
// 判据 = store 的 steerAbortExpected 预告（插话发送时置位为会话 id）：
//   - 预告命中且 finish_reason=abort → lastFinishReason=null（静默）+ 预告消费
//   - 预告命中但 finish_reason=stop（自然收尾）→ 预告失效消费，正常设 stop
//   - 无预告的 abort（真手动停止）→ lastFinishReason=abort（照常提示）
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { listen } from "@tauri-apps/api/event";
import { useChatEvents } from "../useChatEvents";
import { useChatStore } from "../../stores/chat";

/** 捕获 useChatEvents 注册的事件 handler（镜像 approval-notify 测试的 listen mock） */
type ListenFn = typeof listen;
const handlers = new Map<string, (e: { payload: unknown }) => void>();
vi.mocked(listen).mockImplementation(
  async (event: Parameters<ListenFn>[0], handler: Parameters<ListenFn>[1]) => {
    handlers.set(String(event), handler as unknown as (e: { payload: unknown }) => void);
    return () => {
      handlers.delete(String(event));
    };
  },
);

function donePayload(finishReason: string) {
  return { conversation_id: "c1", message_id: "m1", finish_reason: finishReason };
}

describe("Steer 插话打断：chat:done(abort) 静默衔接", () => {
  let cleanup: () => void;

  beforeEach(async () => {
    setActivePinia(createPinia());
    handlers.clear();
    cleanup = await useChatEvents();
    await new Promise((r) => setTimeout(r, 0));
  });
  afterEach(() => {
    cleanup();
  });

  it("预告命中 + abort → lastFinishReason=null（不显「已手动停止」）且预告消费", () => {
    const chat = useChatStore();
    chat.activeConvId = "c1";
    chat.messages = [];
    chat.steerAbortExpected = "c1"; // 插话发送时置位的预告

    handlers.get("chat:done")!({ payload: donePayload("abort") });

    expect(chat.lastFinishReason).toBeNull(); // 静默衔接
    expect(chat.steerAbortExpected).toBeNull(); // 预告消费，防残留
  });

  it("预告命中但自然收尾（stop）→ 预告失效消费，正常设 stop", () => {
    const chat = useChatStore();
    chat.activeConvId = "c1";
    chat.messages = [];
    chat.steerAbortExpected = "c1";

    handlers.get("chat:done")!({ payload: donePayload("stop") });

    expect(chat.lastFinishReason).toBe("stop"); // 非 abort 不静默
    expect(chat.steerAbortExpected).toBeNull();
  });

  it("无预告的 abort（真手动停止）→ lastFinishReason=abort 照常提示", () => {
    const chat = useChatStore();
    chat.activeConvId = "c1";
    chat.messages = [];
    chat.steerAbortExpected = null; // 用户主动点停止，无预告

    handlers.get("chat:done")!({ payload: donePayload("abort") });

    expect(chat.lastFinishReason).toBe("abort"); // 保留「已手动停止」
  });

  it("预告指向其他会话（cid 不匹配）→ 不消费，abort 照常提示", () => {
    const chat = useChatStore();
    chat.activeConvId = "c1";
    chat.messages = [];
    chat.steerAbortExpected = "c2"; // 预告是另一个会话的（本会话的完成与它无关）

    handlers.get("chat:done")!({ payload: donePayload("abort") });

    expect(chat.lastFinishReason).toBe("abort");
    expect(chat.steerAbortExpected).toBe("c2"); // 未消费
  });
});
