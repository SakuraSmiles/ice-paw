// systemNotify.test.ts — 审批系统通知的行为锁定：
// 前台聚焦不发（弹窗本身可见，勿重复打扰）/ 失焦才走权限链发送 /
// 权限拒绝不发 / 链路任何异常静默（通知是旁路，不阻塞审批主流程）。
//
// ⚠️ hasFocus spy 写法：mockRestore() 会把 spy 从属性上摘除（恢复原始
// descriptor），事后对旧 spy 引用再 mockReturnValue 是设置了个寂寞——
// 必须每用例重新 vi.spyOn（这里在 beforeEach 重建，勿提出去模块级复用）。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import type { MockInstance } from "vitest";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { notifyApprovalNeeded } from "../utils/systemNotify";

// factory 直接给好默认实现（已授权），用例按需覆盖——避开 vitest 2.x
// mockReset 对 factory mock「恢复原始实现」的语义坑
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(true),
  requestPermission: vi.fn().mockResolvedValue("granted"),
  sendNotification: vi.fn(),
}));

const mockGranted = vi.mocked(isPermissionGranted);
const mockRequest = vi.mocked(requestPermission);
const mockSend = vi.mocked(sendNotification);

describe("notifyApprovalNeeded 审批系统通知", () => {
  let hasFocusSpy: MockInstance;

  beforeEach(() => {
    hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    mockGranted.mockClear().mockResolvedValue(true);
    mockRequest.mockClear().mockResolvedValue("granted");
    mockSend.mockClear();
  });
  afterEach(() => hasFocusSpy.mockRestore());

  it("前台聚焦时不发（审批弹窗本身可见）", async () => {
    hasFocusSpy.mockReturnValue(true);
    await notifyApprovalNeeded("IcePaw · 等待你的批准", "write_file：修改配置");
    expect(mockGranted).not.toHaveBeenCalled();
    expect(mockSend).not.toHaveBeenCalled();
  });

  it("失焦 + 已授权 → 发送（title/body 原样，不重复请求权限）", async () => {
    await notifyApprovalNeeded("IcePaw · 等待你的批准", "write_file：修改配置");
    expect(mockRequest).not.toHaveBeenCalled();
    expect(mockSend).toHaveBeenCalledTimes(1);
    expect(mockSend).toHaveBeenCalledWith({ title: "IcePaw · 等待你的批准", body: "write_file：修改配置" });
  });

  it("失焦 + 未授权 + 请求后 granted → 发送", async () => {
    mockGranted.mockResolvedValue(false);
    mockRequest.mockResolvedValue("granted");
    await notifyApprovalNeeded("IcePaw · 收到配置提案", "创建 agent：文档助手");
    expect(mockRequest).toHaveBeenCalledTimes(1);
    expect(mockSend).toHaveBeenCalledWith({ title: "IcePaw · 收到配置提案", body: "创建 agent：文档助手" });
  });

  it("失焦 + 权限被拒 → 不发送", async () => {
    mockGranted.mockResolvedValue(false);
    mockRequest.mockResolvedValue("denied");
    await notifyApprovalNeeded("t", "b");
    expect(mockSend).not.toHaveBeenCalled();
  });

  it("通知链路异常（权限查询/发送抛错）→ 静默不炸", async () => {
    mockGranted.mockRejectedValue(new Error("plugin not ready"));
    await expect(notifyApprovalNeeded("t", "b")).resolves.toBeUndefined();

    mockGranted.mockResolvedValue(true);
    mockSend.mockImplementation(() => {
      throw new Error("toast failed");
    });
    await expect(notifyApprovalNeeded("t", "b")).resolves.toBeUndefined();
  });
});
