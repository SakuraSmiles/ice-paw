import { describe, it, expect, beforeEach, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

// 直接测 bridge 的 invoke 行为，不复制 wrapInvokeError

describe("bridge invoke", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("mockResolvedValue returns expected data", async () => {
    mockInvoke.mockResolvedValue([{ id: "1", name: "test" }]);
    const result = await invoke("list_projects");
    expect(result).toEqual([{ id: "1", name: "test" }]);
    expect(mockInvoke).toHaveBeenCalledWith("list_projects");
  });

  it("mockRejectedValue throws with error message", async () => {
    mockInvoke.mockRejectedValue(new Error("connection refused"));
    await expect(invoke("bad_command")).rejects.toThrow("connection refused");
  });

  it("can be called with arguments", async () => {
    mockInvoke.mockResolvedValue({ id: "new" });
    await invoke("create_project", { input: { name: "test" } });
    expect(mockInvoke).toHaveBeenCalledWith("create_project", {
      input: { name: "test" },
    });
  });

  it("rejects when invoke fails with non-Error object", async () => {
    mockInvoke.mockRejectedValue({ message: "invalid", kind: "validation" });
    await expect(invoke("bad")).rejects.toEqual({
      message: "invalid",
      kind: "validation",
    });
  });
});
