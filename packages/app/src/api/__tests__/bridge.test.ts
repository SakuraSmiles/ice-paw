import { describe, it, expect, beforeEach, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { bridge } from "../bridge";

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

describe("bridge.messages.listByTurns", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("命令名与参数透传（游标原样回传）+ 返回形状", async () => {
    const page = { rows: [], has_more: false };
    mockInvoke.mockResolvedValue(page);
    const r = await bridge.messages.listByTurns("c1", { beforeAnchorRowid: 9 });
    expect(mockInvoke).toHaveBeenCalledWith("list_messages_by_turns", {
      conversationId: "c1",
      turns: undefined,
      beforeAnchorRowid: 9,
    });
    expect(r).toEqual(page);
  });

  it("invoke 失败经 wrapInvokeError 包装（messages.listByTurns 前缀）", async () => {
    mockInvoke.mockRejectedValue(new Error("boom"));
    await expect(bridge.messages.listByTurns("c1")).rejects.toThrow(
      "[bridge.messages.listByTurns] boom",
    );
  });
});
