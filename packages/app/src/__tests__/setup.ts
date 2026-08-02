// 前端测试全局 Setup：mock Tauri 运行时依赖
import { vi } from "vitest";

// Stub Tauri invoke —— 每个测试按需 mockResolvedValue/mockRejectedValue
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

// Stub Tauri event —— 测试中通过 vi.mocked(listen) 捕获回调
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
  emit: vi.fn().mockResolvedValue(undefined),
}));

// 静默测试中的 console.error（避免干扰输出）
vi.spyOn(console, "error").mockImplementation(() => {});
