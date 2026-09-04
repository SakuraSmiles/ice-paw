// useChatEvents.approval-notify.test.ts — 审批系统通知「恰一次」语义锁定：
// 触发条件 = 请求存活期间应用失焦。到达时已失焦立即发；到达时聚焦（用户正
// 盯着审批卡片）则首次 blur 补发——生产实案 2026-09-03：dev 实测盯着卡片
// 出来再切后台，旧「到达瞬间失焦才发」一条通知都没有。
// 同一 request_id 多触发源（事件到达 + blur + visibilitychange）不重发；
// destroy 卸载 DOM 监听。
// 发送链路 = bridge.chat.notifyApproval（Rust 命令 harness/approval_toast：
// Windows toast 带批准/拒绝按钮）；工具授权透传 request_id（按钮语义），
// 提案不传（纯提醒）。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { listen } from "@tauri-apps/api/event";
import { bridge } from "../../api/bridge";
import type { MockInstance } from "vitest";
import { useChatEvents } from "../useChatEvents";
import { useChatStore } from "../../stores/chat";
import type { ToolAuthRequestPayload } from "../../types";

vi.mock("../../api/bridge", () => ({
  bridge: { chat: { notifyApproval: vi.fn().mockResolvedValue(undefined) } },
}));

const mockNotify = vi.mocked(bridge.chat.notifyApproval);

/** 捕获 useChatEvents 注册的事件 handler（setup.ts 的 listen mock 被 override） */
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

function authPayload(requestId: string): ToolAuthRequestPayload {
  return {
    request_id: requestId,
    tool_use_id: "t1",
    tool_name: "write_file",
    file_path: "D:/tmp/x.md",
    arguments: "{}",
    conversation_id: "c1",
    message_id: "m1",
    reason: "写入应用配置",
  };
}

/** 委派授权 payload（chat:delegation-auth-request；agent_name 字段为判别键） */
function delegationPayload(requestId: string) {
  return {
    request_id: requestId,
    conversation_id: "c1",
    message_id: "m1",
    agent_name: "wenshu",
    agent_id: "a-wenshu",
    task: "整理需求文档并生成报告",
  };
}

describe("审批系统通知：恰一次语义", () => {
  let hasFocusSpy: MockInstance;
  let cleanup: () => void;

  beforeEach(async () => {
    setActivePinia(createPinia());
    handlers.clear();
    mockNotify.mockClear().mockResolvedValue(undefined);
    hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    cleanup = await useChatEvents();
    // dev 通路自检通知（useChatEvents 内 DEV 分支）落地后再清计数，
    // 不让它污染各用例的「恰一次」断言
    await new Promise((r) => setTimeout(r, 0));
    mockNotify.mockClear();
  });
  afterEach(() => {
    cleanup();
    hasFocusSpy.mockRestore();
  });

  it("到达时已失焦 → 立即一条；再 blur 不重发", async () => {
    handlers.get("chat:tool-auth-request")!({ payload: authPayload("r1") });
    await Promise.resolve();
    expect(mockNotify).toHaveBeenCalledTimes(1);
    expect(mockNotify).toHaveBeenCalledWith({
      title: "IcePaw · 等待你的批准",
      body: "write_file：写入应用配置",
      request_id: "r1",
    });

    window.dispatchEvent(new Event("blur"));
    await Promise.resolve();
    expect(mockNotify).toHaveBeenCalledTimes(1); // 恰一次
  });

  it("到达时聚焦不发；随后切走（blur）补发一条——生产实案路径", async () => {
    hasFocusSpy.mockReturnValue(true); // 用户正盯着应用，卡片刚出来
    handlers.get("chat:tool-auth-request")!({ payload: authPayload("r1") });
    await Promise.resolve();
    expect(mockNotify).not.toHaveBeenCalled();

    hasFocusSpy.mockReturnValue(false); // 切后台
    window.dispatchEvent(new Event("blur"));
    await Promise.resolve();
    expect(mockNotify).toHaveBeenCalledTimes(1);
  });

  it("新请求（新 request_id）在挂起期间到达 → blur 时各发各的", async () => {
    handlers.get("chat:tool-auth-request")!({ payload: authPayload("r1") });
    window.dispatchEvent(new Event("blur"));
    await Promise.resolve();
    expect(mockNotify).toHaveBeenCalledTimes(1); // r1 已通知，不重发

    handlers.get("chat:tool-auth-request")!({ payload: authPayload("r2") });
    window.dispatchEvent(new Event("blur"));
    await Promise.resolve();
    expect(mockNotify).toHaveBeenCalledTimes(2); // r2 首次
  });

  it("最小化（visibilitychange hidden）同样触发补发", async () => {
    hasFocusSpy.mockReturnValue(true);
    handlers.get("chat:tool-auth-request")!({ payload: authPayload("r1") });
    await Promise.resolve();
    expect(mockNotify).not.toHaveBeenCalled();

    hasFocusSpy.mockReturnValue(false);
    // happy-dom 手动驱动 visibilitychange
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    await Promise.resolve();
    expect(mockNotify).toHaveBeenCalledTimes(1);
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
  });

  it("destroy 后 blur 不再触发（teardown 干净）", async () => {
    cleanup();
    // 事件 handler 已随 destroy 拆卸（handlers Map 清空）；直接塞 store 模拟挂起审批
    const chat = useChatStore();
    const m = new Map(chat.pendingAuthRequests);
    m.set("c1", { payload: authPayload("r9"), receivedAt: Date.now() });
    chat.pendingAuthRequests = m;
    window.dispatchEvent(new Event("blur"));
    await Promise.resolve();
    expect(mockNotify).not.toHaveBeenCalled();
  });

  it("toast 按钮应答（chat:tool-auth-responded）→ 前端条目同步清除", async () => {
    handlers.get("chat:tool-auth-request")!({ payload: authPayload("r1") });
    const chat = useChatStore();
    expect(chat.pendingAuthRequests.size).toBe(1);

    // Rust 侧 approval_toast::respond_tool_auth_and_emit 广播（toast 按钮路径
    // 前端无乐观删，全靠此事件——无它则卡片残留到 120s 超时）
    handlers.get("chat:tool-auth-responded")!({ payload: { request_id: "r1", allowed: true } });
    await Promise.resolve();
    expect(chat.pendingAuthRequests.size).toBe(0);
  });

  // ---- 委派授权（chat:delegation-auth-request，2026-09-03 委托时预授权）----
  // 与工具授权共用 pendingAuthRequests 栈：进栈 / 失焦通知（文案带目标
  // agent）/ responded 清除 / cancel 清除全部机制继承

  it("委派授权请求 → 进同一挂起栈，失焦通知文案带「委派给」与任务摘要", async () => {
    handlers.get("chat:delegation-auth-request")!({ payload: delegationPayload("d1") });
    await Promise.resolve();
    const chat = useChatStore();
    expect(chat.pendingAuthRequests.size).toBe(1);

    await new Promise((r) => setTimeout(r, 0)); // fire-and-forget 通知落定
    expect(mockNotify).toHaveBeenCalledTimes(1);
    expect(mockNotify).toHaveBeenCalledWith({
      title: "IcePaw · 等待你的批准",
      body: "委派给 wenshu：整理需求文档并生成报告",
      request_id: "d1",
    });
  });

  it("委派条目同样被 tool-auth-responded 清除（toast 批准路径）", async () => {
    handlers.get("chat:delegation-auth-request")!({ payload: delegationPayload("d2") });
    await Promise.resolve();
    const chat = useChatStore();
    expect(chat.pendingAuthRequests.size).toBe(1);

    handlers.get("chat:tool-auth-responded")!({ payload: { request_id: "d2", allowed: true } });
    await Promise.resolve();
    expect(chat.pendingAuthRequests.size).toBe(0);
  });

  it("委派条目被 tool-auth-request-cancel 清除（后端 120s 超时路径）", async () => {
    handlers.get("chat:delegation-auth-request")!({ payload: delegationPayload("d3") });
    await Promise.resolve();
    const chat = useChatStore();
    expect(chat.pendingAuthRequests.size).toBe(1);

    handlers.get("chat:tool-auth-request-cancel")!({ payload: { request_id: "d3" } });
    await Promise.resolve();
    expect(chat.pendingAuthRequests.size).toBe(0);
  });
});
