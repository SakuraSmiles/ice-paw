// systemNotify.test.ts — 审批系统通知的行为锁定：
// 前台聚焦不发（弹窗本身可见，勿重复打扰）/ 失焦才走 Rust 命令发送（带按钮
// 语义的 request_id 透传）/ 链路任何异常静默（通知是旁路，不阻塞审批主流程）。
//
// ⚠️ hasFocus spy 写法：mockRestore() 会把 spy 从属性上摘除（恢复原始
// descriptor），事后对旧 spy 引用再 mockReturnValue 是设置了个寂寞——
// 必须每用例重新 vi.spyOn（这里在 beforeEach 重建，勿提出去模块级复用）。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import type { MockInstance } from "vitest";
import { bridge } from "../api/bridge";
import { notifyApprovalNeeded, notifySelfCheck } from "../utils/systemNotify";

// 发送链路已从 JS plugin 通道迁到 Rust 命令（harness/approval_toast：Windows
// toast 带批准/拒绝按钮）——mock bridge 层即可，无需再 mock plugin 权限链
vi.mock("../api/bridge", () => ({
  bridge: { chat: { notifyApproval: vi.fn().mockResolvedValue(undefined) } },
}));

const mockNotify = vi.mocked(bridge.chat.notifyApproval);

describe("notifyApprovalNeeded 审批系统通知", () => {
  let hasFocusSpy: MockInstance;

  beforeEach(() => {
    hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    mockNotify.mockClear().mockResolvedValue(undefined);
  });
  afterEach(() => hasFocusSpy.mockRestore());

  it("前台聚焦时不发（审批弹窗本身可见）", async () => {
    hasFocusSpy.mockReturnValue(true);
    await notifyApprovalNeeded("IcePaw · 等待你的批准", "write_file：修改配置", "r1");
    expect(mockNotify).not.toHaveBeenCalled();
  });

  it("失焦 → 发送，工具授权 request_id 原样透传（toast 按钮语义）", async () => {
    await notifyApprovalNeeded("IcePaw · 等待你的批准", "write_file：修改配置", "r1");
    expect(mockNotify).toHaveBeenCalledTimes(1);
    expect(mockNotify).toHaveBeenCalledWith({
      title: "IcePaw · 等待你的批准",
      body: "write_file：修改配置",
      request_id: "r1",
    });
  });

  it("提案不传 request_id → 字段 undefined（纯提醒，无按钮）", async () => {
    await notifyApprovalNeeded("IcePaw · 收到配置提案", "创建 agent：文档助手");
    expect(mockNotify).toHaveBeenCalledWith({
      title: "IcePaw · 收到配置提案",
      body: "创建 agent：文档助手",
      request_id: undefined,
    });
  });

  it("通知链路异常 → 静默不炸（通知是旁路）", async () => {
    // mockImplementation 惰性创建 rejected promise——mockRejectedValue 会立即
    // 造一个 rejection，若未被消费（reset 时序）即 unhandled 报错
    mockNotify.mockImplementation(() => Promise.reject(new Error("命令缺失")));
    await expect(notifyApprovalNeeded("t", "b")).resolves.toBeUndefined();
  });
});

describe("notifySelfCheck dev 通路自检", () => {
  beforeEach(() => mockNotify.mockClear().mockResolvedValue(undefined));

  it("跳过焦点判定直发，带 fake request_id（dev 看按钮样式）", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true); // 聚焦也发——自检不受守卫
    await notifySelfCheck();
    expect(mockNotify).toHaveBeenCalledTimes(1);
    const arg = mockNotify.mock.calls[0][0];
    expect(arg.title).toBe("IcePaw 通知自检");
    expect(arg.request_id).toMatch(/^self-check-/);
  });
});

// 失败注入独立 describe（勿挂进带 beforeEach 的块）：vitest 4.1 mock 怪癖——
// beforeEach 触碰过的 mock（mockClear/mockResolvedValue）上再 mockImplementation
// 注入失败（rejected promise 或同步 throw 都一样），会被归因为用例失败
// （Error 指向注入点、无断言 diff）；不触碰 mock 状态的块内注入则正常。
describe("notifySelfCheck 发送失败", () => {
  it("静默不炸", async () => {
    mockNotify.mockImplementation(() => Promise.reject(new Error("toast failed")));
    await expect(notifySelfCheck()).resolves.toBeUndefined();
  });
});
