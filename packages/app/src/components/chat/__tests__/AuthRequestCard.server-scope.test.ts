// AuthRequestCard.server-scope.test.ts — ③ 此 Server（本会话）第三档（2026-09-11）：
// 外部 server 工具的授权请求带 server_name → 卡片多出「此 Server（本会话）」档 +
// L2 来源标注；选中后允许按 this_server 回传。内置工具（无 server_name）不显示该档。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import AuthRequestCard from "../AuthRequestCard.vue";
import { useChatStore } from "../../../stores/chat";
import type { ToolAuthRequestPayload } from "../../../types";

function authPayload(overrides: Partial<ToolAuthRequestPayload> = {}): ToolAuthRequestPayload {
  return {
    request_id: "req-1",
    tool_use_id: "tu-1",
    tool_name: "t3_spawn_actor",
    file_path: "",
    arguments: "{}",
    conversation_id: "c1",
    message_id: "m1",
    reason: "此工具需要用户确认授权",
    ...overrides,
  };
}

function mountCard(payload: ToolAuthRequestPayload) {
  const chat = useChatStore();
  chat.activeConvId = "c1";
  chat.pendingAuthRequests = new Map([
    ["c1", { payload, receivedAt: Date.now() }],
  ]);
  return mount(AuthRequestCard, { attachTo: document.body });
}

function scopeLabels(wrapper: ReturnType<typeof mount>) {
  return wrapper.findAll(".auth-scope-opt").map((b) => b.text());
}

describe("AuthRequestCard 此 Server 档（③ 会话级 server 信任）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    document.body.innerHTML = "";
  });

  it("内置工具（无 server_name）：不显示此 Server 档；无路径隐藏此目录档", () => {
    const wrapper = mountCard(authPayload());
    const labels = scopeLabels(wrapper);
    expect(labels).toContain("仅此一次");
    expect(labels).toContain("此工具（本会话）");
    expect(labels).not.toContain("此目录（含子目录）");
    expect(labels).not.toContain("此 Server（本会话）");
    expect(wrapper.text()).not.toContain("Server:");
  });

  it("外部 server 工具：多出此 Server 档 + L2 来源标注", () => {
    const wrapper = mountCard(authPayload({ server_name: "UE 编辑器 MCP" }));
    const labels = scopeLabels(wrapper);
    expect(labels).toContain("此 Server（本会话）");
    expect(wrapper.text()).toContain("Server: UE 编辑器 MCP");
  });

  it("有路径 + server_name：四档全列", () => {
    const wrapper = mountCard(
      authPayload({ server_name: "UE 编辑器 MCP", file_path: "D:/proj/x.uasset" }),
    );
    const labels = scopeLabels(wrapper);
    expect(labels).toEqual([
      "仅此一次",
      "此目录（含子目录）",
      "此工具（本会话）",
      "此 Server（本会话）",
    ]);
  });

  it("选此 Server 档后允许：respondToAuth 带 this_server", async () => {
    const chat = useChatStore();
    const spy = vi.spyOn(chat, "respondToAuth").mockResolvedValue(undefined);
    const wrapper = mountCard(authPayload({ server_name: "UE 编辑器 MCP" }));
    const serverOpt = wrapper
      .findAll(".auth-scope-opt")
      .find((b) => b.text() === "此 Server（本会话）")!;
    await serverOpt.trigger("click");
    await wrapper.find(".auth-btn-allow").trigger("click");
    expect(spy).toHaveBeenCalledWith("req-1", true, "this_server", undefined);
  });

  it("默认档仍是仅此一次（server 档是显式选择，不默认扩大信任面）", async () => {
    const chat = useChatStore();
    const spy = vi.spyOn(chat, "respondToAuth").mockResolvedValue(undefined);
    const wrapper = mountCard(authPayload({ server_name: "UE 编辑器 MCP" }));
    await wrapper.find(".auth-btn-allow").trigger("click");
    expect(spy).toHaveBeenCalledWith("req-1", true, "once", undefined);
  });
});
