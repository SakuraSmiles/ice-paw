import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, type VueWrapper } from "@vue/test-utils";
import AuthRequestCard from "../AuthRequestCard.vue";
import AuthNoticeStack from "../AuthNoticeStack.vue";
import { useChatStore } from "../../../stores/chat";
import type { ToolAuthRequestPayload } from "../../../types";

vi.mock("vue-router", () => ({ useRouter: () => ({ push: vi.fn() }) }));

function request(overrides: Partial<ToolAuthRequestPayload> = {}): ToolAuthRequestPayload {
  return {
    request_id: "req-1", tool_use_id: "tu-1", tool_name: "copy_file",
    file_path: "D:/outside/output.txt", arguments: "{}", conversation_id: "c1", message_id: "m1",
    reason: "部分路径需要授权",
    paths: [
      { key: "source", path: "D:/workspace/input.txt", requires_approval: false },
      { key: "destination", path: "D:/outside/output.txt", requires_approval: true },
    ],
    directory_path: "D:/outside",
    ...overrides,
  };
}

const wrappers: VueWrapper[] = [];
function mountRequest(payload: ToolAuthRequestPayload, background = false) {
  const chat = useChatStore();
  chat.activeConvId = background ? "other-conversation" : "c1";
  chat.pendingAuthRequests = new Map([["c1", { payload, receivedAt: Date.now() }]]);
  const w = mount(background ? AuthNoticeStack : AuthRequestCard, { attachTo: document.body });
  wrappers.push(w);
  return w;
}
function directoryOption(w: VueWrapper) {
  return w.findAll(".auth-scope-opt").find((b) => b.text() === "此目录（含子目录）")!;
}

beforeEach(() => { setActivePinia(createPinia()); });
afterEach(() => { while (wrappers.length) wrappers.pop()?.unmount(); });

describe("内联授权卡多路径展示", () => {
  it("来源已授权、目标待授权：两端直接显示，默认批准仍为 once", async () => {
    const respond = vi.spyOn(useChatStore(), "respondToAuth").mockResolvedValue(undefined);
    const w = mountRequest(request());
    const rows = w.findAll(".auth-path-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("来源");
    expect(rows[0].find(".auth-path-value").text()).toBe("D:/workspace/input.txt");
    expect(rows[0].find(".auth-path-status").text()).toBe("已授权");
    expect(rows[1].text()).toContain("目标");
    expect(rows[1].find(".auth-path-value").text()).toBe("D:/outside/output.txt");
    expect(rows[1].find(".auth-path-status").text()).toBe("待授权");
    expect(directoryOption(w).attributes("title")).toBe("D:/outside");
    await w.find(".auth-btn-allow").trigger("click");
    expect(respond).toHaveBeenCalledWith("req-1", true, "once", undefined);
  });

  it("双未授权：两条都标待授权，此目录只展示后端指定目录且不改变响应协议", async () => {
    const respond = vi.spyOn(useChatStore(), "respondToAuth").mockResolvedValue(undefined);
    const w = mountRequest(request({
      tool_name: "move_file", file_path: "D:/source/input.txt", directory_path: "D:/source",
      paths: [
        { key: "source", path: "D:/source/input.txt", requires_approval: true },
        { key: "destination", path: "D:/destination/output.txt", requires_approval: true },
      ],
    }));
    expect(w.findAll(".auth-path-status").map((s) => s.text())).toEqual(["待授权", "待授权"]);
    await directoryOption(w).trigger("click");
    const hint = w.find(".auth-directory-hint");
    expect(hint.find("code").text()).toBe("D:/source");
    expect(hint.text()).toContain("本会话仅记住此目录（含子目录）");
    expect(hint.text()).toContain("其余路径仅批准本次操作");
    expect(hint.text()).not.toContain("D:/destination");
    await w.find(".auth-btn-allow").trigger("click");
    expect(respond).toHaveBeenCalledWith("req-1", true, "this_dir", undefined);
  });

  it("只授权目标目录时按 directory_path 展示，不从来源或参数猜目录", async () => {
    const w = mountRequest(request({ directory_path: "D:/resolved-target" }));
    await directoryOption(w).trigger("click");
    expect(w.find(".auth-directory-hint code").text()).toBe("D:/resolved-target");
    expect(w.find(".auth-directory-hint").text()).not.toContain("D:/workspace");
  });

  it("普通单路径保持紧凑，同时显示实际目录档范围", async () => {
    const w = mountRequest(request({
      tool_name: "write_file", file_path: "D:/outside/output.txt",
      paths: [{ key: "path", path: "D:/outside/output.txt", requires_approval: true }],
    }));
    expect(w.find(".auth-paths").exists()).toBe(false);
    expect(w.find(".auth-path").text()).toBe("D:/outside/output.txt");
    await directoryOption(w).trigger("click");
    expect(w.find(".auth-directory-hint code").text()).toBe("D:/outside");
    expect(w.find(".auth-directory-hint").text()).not.toContain("其余路径");
  });

  it("旧 payload 无 paths/directory_path 时保持路径展示与范围响应", async () => {
    const respond = vi.spyOn(useChatStore(), "respondToAuth").mockResolvedValue(undefined);
    const w = mountRequest(request({ paths: undefined, directory_path: undefined }));
    expect(w.find(".auth-path").text()).toBe("D:/outside/output.txt");
    expect(w.find(".auth-paths").exists()).toBe(false);
    await directoryOption(w).trigger("click");
    expect(w.find(".auth-directory-hint").exists()).toBe(false);
    await w.find(".auth-btn-allow").trigger("click");
    expect(respond).toHaveBeenCalledWith("req-1", true, "this_dir", undefined);
  });
});

describe("后台授权通知栈多路径展示", () => {
  it("两端路径及各自授权状态可见，后台允许仍固定 once", async () => {
    const respond = vi.spyOn(useChatStore(), "respondToAuth").mockResolvedValue(undefined);
    const w = mountRequest(request(), true);
    const rows = w.findAll(".notice-path-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("来源");
    expect(rows[0].find(".notice-path-value").text()).toBe("D:/workspace/input.txt");
    expect(rows[0].find(".notice-path-status").text()).toBe("已授权");
    expect(rows[1].text()).toContain("目标");
    expect(rows[1].find(".notice-path-value").text()).toBe("D:/outside/output.txt");
    expect(rows[1].find(".notice-path-status").text()).toBe("待授权");
    await w.find(".notice-btn-allow").trigger("click");
    expect(respond).toHaveBeenCalledWith("req-1", true, "once");
  });

  it("双未授权不因后台简化而遗漏来源", () => {
    const w = mountRequest(request({
      paths: [
        { key: "source", path: "D:/source/input.txt", requires_approval: true },
        { key: "destination", path: "D:/destination/output.txt", requires_approval: true },
      ],
    }), true);
    expect(w.findAll(".notice-path-status").map((s) => s.text())).toEqual(["待授权", "待授权"]);
    expect(w.text()).toContain("D:/source/input.txt");
    expect(w.text()).toContain("D:/destination/output.txt");
  });

  it("兼容旧 payload 单路径通知，拒绝响应保持不变", async () => {
    const respond = vi.spyOn(useChatStore(), "respondToAuth").mockResolvedValue(undefined);
    const w = mountRequest(request({ paths: undefined, directory_path: undefined }), true);
    expect(w.find(".notice-path").text()).toBe("D:/outside/output.txt");
    expect(w.find(".notice-paths").exists()).toBe(false);
    await w.find(".notice-btn-deny").trigger("click");
    expect(respond).toHaveBeenCalledWith("req-1", false);
  });
});
