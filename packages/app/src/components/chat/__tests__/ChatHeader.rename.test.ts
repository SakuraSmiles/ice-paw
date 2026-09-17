// ChatHeader 标题行内改名——显式保存契约回归锁（U0-6）。
// 锁定行为：
// - 草稿态：双击标题进入编辑，保存/取消按钮在场；blur 不再即存（旧契约摘除）
// - 显式保存：点「保存」/Enter → rename 调用 + 标题更新 + 「已保存」短驻淡出
// - 失败可见：rename 拒绝 → 编辑态保持 + 三段式文案贴身 + 标题不变 + 可重试
// - 取消回滚：点「取消」/Escape → 零调用收起，草稿回滚
// - 空标题双闸：保存按钮禁用；兜底提示不静默回滚
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import ChatHeader from "../ChatHeader.vue";
import { useChatStore } from "../../../stores/chat";
import { bridge } from "../../../api/bridge";

vi.mock("../../../api/bridge", () => ({
  bridge: {
    conversations: {
      rename: vi.fn().mockResolvedValue(undefined),
    },
  },
}));

const mockRename = vi.mocked(bridge.conversations.rename);

function conv(id: string) {
  return {
    id,
    agent_id: "a1",
    title: "测试对话",
    pinned: false,
    created_at: "2026-08-15 00:00:00",
    updated_at: "2026-08-15 00:00:00",
    project_id: null,
  };
}

async function mountEditing() {
  const chat = useChatStore();
  chat.conversations = [conv("c1")];
  chat.selectConversation("c1");
  const wrapper = mount(ChatHeader, { attachTo: document.body });
  await Promise.resolve();
  wrapper.find(".header-title").trigger("dblclick");
  await Promise.resolve();
  return { chat, wrapper };
}

describe("ChatHeader 标题行内改名（显式保存契约）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    document.body.innerHTML = "";
    mockRename.mockReset().mockResolvedValue(undefined);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("双击进入编辑：草稿态控件在场（输入框 + 保存 + 取消）", async () => {
    const { wrapper } = await mountEditing();
    expect(wrapper.find(".header-edit-input").exists()).toBe(true);
    expect(wrapper.find(".title-edit-btn.is-save").exists()).toBe(true);
    expect(wrapper.find(".title-edit-btn:not(.is-save)").text()).toBe("取消");
    // 草稿初值 = 当前标题
    expect((wrapper.find(".header-edit-input").element as HTMLInputElement).value).toBe("测试对话");
  });

  it("blur 不再即存：输入框失焦后编辑态保持、零调用", async () => {
    const { wrapper } = await mountEditing();
    const input = wrapper.find(".header-edit-input");
    (input.element as HTMLInputElement).value = "改名草案";
    input.trigger("input");
    input.element.dispatchEvent(new FocusEvent("blur"));
    await Promise.resolve();
    expect(mockRename).not.toHaveBeenCalled();
    expect(wrapper.find(".header-edit-row").exists()).toBe(true); // 草稿仍在场
  });

  it("显式保存：rename 调用 + 标题更新 + 「已保存」短驻淡出", async () => {
    vi.useFakeTimers();
    const { chat, wrapper } = await mountEditing();
    const input = wrapper.find(".header-edit-input");
    (input.element as HTMLInputElement).value = "新标题";
    input.trigger("input");
    wrapper.find(".title-edit-btn.is-save").trigger("click");
    await vi.advanceTimersByTimeAsync(0); // 等 rename 微任务链
    expect(mockRename).toHaveBeenCalledWith("c1", "新标题");
    expect(chat.activeConversation?.title).toBe("新标题");
    expect(wrapper.find(".header-edit-row").exists()).toBe(false); // 收起
    expect(wrapper.find(".title-saved-flag").exists()).toBe(true); // 已保存在场
    await vi.advanceTimersByTimeAsync(1700);
    expect(wrapper.find(".title-saved-flag").exists()).toBe(false); // 淡出
  });

  it("Enter 保存同款（IME 组合期回车不触发）", async () => {
    const { wrapper } = await mountEditing();
    const input = wrapper.find(".header-edit-input");
    (input.element as HTMLInputElement).value = "回车改名";
    input.trigger("input");
    input.trigger("keydown", { key: "Enter" });
    await Promise.resolve();
    await Promise.resolve();
    expect(mockRename).toHaveBeenCalledWith("c1", "回车改名");

    // isComposing=true（输入法选词）不保存
    mockRename.mockClear();
    const second = await mountEditing();
    const input2 = second.wrapper.find(".header-edit-input");
    (input2.element as HTMLInputElement).value = "拼音中";
    input2.trigger("input");
    input2.trigger("keydown", { key: "Enter", isComposing: true } as KeyboardEventInit);
    await Promise.resolve();
    expect(mockRename).not.toHaveBeenCalled();
  });

  it("保存失败可见：编辑态保持 + 错误文案在场 + 标题不变 + 可重试", async () => {
    mockRename.mockRejectedValueOnce(new Error("数据库被锁定：另一进程占用——请重试或稍后再试"));
    const { chat, wrapper } = await mountEditing();
    const input = wrapper.find(".header-edit-input");
    (input.element as HTMLInputElement).value = "会失败的改名";
    input.trigger("input");
    wrapper.find(".title-edit-btn.is-save").trigger("click");
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(wrapper.find(".header-edit-row").exists()).toBe(true); // 不收起
    expect(wrapper.find(".title-edit-error").text()).toContain("数据库被锁定");
    expect(chat.activeConversation?.title).toBe("测试对话");
    // 重试（第二次放行）→ 成功收起
    wrapper.find(".title-edit-btn.is-save").trigger("click");
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(mockRename).toHaveBeenCalledTimes(2);
    expect(wrapper.find(".header-edit-row").exists()).toBe(false);
    expect(chat.activeConversation?.title).toBe("会失败的改名");
  });

  it("取消回滚：零调用收起；Escape 同款", async () => {
    const { chat, wrapper } = await mountEditing();
    const input = wrapper.find(".header-edit-input");
    (input.element as HTMLInputElement).value = "将被丢弃的草案";
    input.trigger("input");
    wrapper.find(".title-edit-btn:not(.is-save)").trigger("click");
    await Promise.resolve();
    expect(mockRename).not.toHaveBeenCalled();
    expect(wrapper.find(".header-edit-row").exists()).toBe(false);
    expect(chat.activeConversation?.title).toBe("测试对话");

    // Esc 路径：重新进入 → 改稿 → Escape 收起零调用
    wrapper.find(".header-title").trigger("dblclick");
    await Promise.resolve();
    const input2 = wrapper.find(".header-edit-input");
    (input2.element as HTMLInputElement).value = "Esc 草案";
    input2.trigger("input");
    input2.trigger("keydown", { key: "Escape" });
    await Promise.resolve();
    expect(mockRename).not.toHaveBeenCalled();
    expect(wrapper.find(".header-edit-row").exists()).toBe(false);
  });

  it("空标题双闸：保存按钮禁用 + Enter 兜底提示（不静默回滚）", async () => {
    const { wrapper } = await mountEditing();
    const input = wrapper.find(".header-edit-input");
    (input.element as HTMLInputElement).value = "   ";
    input.trigger("input");
    await Promise.resolve();
    expect((wrapper.find(".title-edit-btn.is-save").element as HTMLButtonElement).disabled).toBe(true);
    // 兜底：绕过 disabled 直接 Enter（防御未来的 UI 改动）
    input.trigger("keydown", { key: "Enter" });
    await Promise.resolve();
    await Promise.resolve();
    expect(mockRename).not.toHaveBeenCalled();
    expect(wrapper.find(".header-edit-row").exists()).toBe(true); // 保持编辑态
    expect(wrapper.find(".title-edit-error").text()).toContain("标题不能为空");
  });
});
