// AskUserCard.test.ts — ask_user 选择卡（2026-09-30）：
// 单选点击即答 / 多选勾选 + 提交（空选禁用）/「其他」自由输入 / 跳过 = dismissed /
// 新请求到达草稿重置。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import AskUserCard from "../AskUserCard.vue";
import { useChatStore } from "../../../stores/chat";
import type { AskUserRequestPayload } from "../../../types";

function askPayload(overrides: Partial<AskUserRequestPayload> = {}): AskUserRequestPayload {
  return {
    request_id: "ask-1",
    conversation_id: "c1",
    tool_use_id: "tu-1",
    question: "用哪个方案？",
    options: [
      { label: "方案 A：直改现有文档", description: "快，但改动不可回退" },
      { label: "方案 B：另存新文档", description: "保留原件" },
      { label: "方案 C：先出草稿再定", description: null },
    ],
    multiple: false,
    allow_custom: true,
    ...overrides,
  };
}

function mountCard(payload: AskUserRequestPayload) {
  const chat = useChatStore();
  chat.activeConvId = "c1";
  chat.pendingAskRequests = new Map([["c1", { payload, receivedAt: Date.now() }]]);
  return mount(AskUserCard, { attachTo: document.body });
}

function respondSpy() {
  const chat = useChatStore();
  return vi.spyOn(chat, "respondToAsk").mockResolvedValue(undefined);
}

describe("AskUserCard 选择卡", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    document.body.innerHTML = "";
  });

  it("单选：点击选项即作答（无确认步），payload 带选项 label", async () => {
    const spy = respondSpy();
    const wrapper = mountCard(askPayload());
    const opts = wrapper.findAll(".ask-opt");
    expect(opts).toHaveLength(3);
    await opts[1].trigger("click");
    expect(spy).toHaveBeenCalledWith("ask-1", "answered", ["方案 B：另存新文档"], null);
  });

  it("多选：勾选两项后提交，selected 按勾选序；空选时提交禁用", async () => {
    const spy = respondSpy();
    const wrapper = mountCard(askPayload({ multiple: true }));
    const opts = wrapper.findAll(".ask-opt");
    const submit = wrapper.find(".ask-btn-primary");
    expect((submit.element as HTMLButtonElement).disabled).toBe(true);

    await opts[0].trigger("click");
    await opts[2].trigger("click");
    // 再点一次取消勾选
    await opts[0].trigger("click");
    await opts[1].trigger("click");
    expect((submit.element as HTMLButtonElement).disabled).toBe(false);
    await submit.trigger("click");
    expect(spy).toHaveBeenCalledWith(
      "ask-1",
      "answered",
      ["方案 C：先出草稿再定", "方案 B：另存新文档"],
      null,
    );
  });

  it("单选「其他」：展开输入后提交，custom_text 传值、selected 为空", async () => {
    const spy = respondSpy();
    const wrapper = mountCard(askPayload());
    await wrapper.find(".ask-other-btn").trigger("click");
    const input = wrapper.find(".ask-input");
    expect(input.exists()).toBe(true);
    await input.setValue("两个都试，先 B");
    await wrapper.find(".ask-submit-btn").trigger("click");
    expect(spy).toHaveBeenCalledWith("ask-1", "answered", [], "两个都试，先 B");
  });

  it("多选 + 自由输入补充：勾选与 custom_text 同批提交", async () => {
    const spy = respondSpy();
    const wrapper = mountCard(askPayload({ multiple: true }));
    await wrapper.findAll(".ask-opt")[0].trigger("click");
    await wrapper.find(".ask-input").setValue("优先保原件");
    await wrapper.find(".ask-btn-primary").trigger("click");
    expect(spy).toHaveBeenCalledWith(
      "ask-1",
      "answered",
      ["方案 A：直改现有文档"],
      "优先保原件",
    );
  });

  it("跳过 → dismissed（agent 收「自行决策并说明」）", async () => {
    const spy = respondSpy();
    const wrapper = mountCard(askPayload());
    await wrapper.find(".ask-btn-skip").trigger("click");
    expect(spy).toHaveBeenCalledWith("ask-1", "dismissed", [], null);
  });

  it("allow_custom=false 不渲染「其他」入口；多选时也不渲染补充输入行", async () => {
    const wrapper = mountCard(askPayload({ allow_custom: false }));
    expect(wrapper.find(".ask-other-btn").exists()).toBe(false);
    const multi = mountCard(askPayload({ allow_custom: false, multiple: true }));
    expect(multi.find(".ask-input").exists()).toBe(false);
  });

  it("新请求到达：勾选与输入草稿重置", async () => {
    respondSpy();
    const chat = useChatStore();
    const wrapper = mountCard(askPayload({ multiple: true }));
    await wrapper.findAll(".ask-opt")[0].trigger("click");
    await wrapper.find(".ask-input").setValue("遗留输入");
    // 新请求（同 conv 覆盖条目）
    chat.pendingAskRequests = new Map([
      ["c1", { payload: askPayload({ request_id: "ask-2", multiple: true }), receivedAt: Date.now() }],
    ]);
    await wrapper.vm.$nextTick();
    // 无勾选（提交禁用）且输入清空
    const submit = wrapper.find(".ask-btn-primary");
    expect((submit.element as HTMLButtonElement).disabled).toBe(true);
    expect((wrapper.find(".ask-input").element as HTMLInputElement).value).toBe("");
  });
});
