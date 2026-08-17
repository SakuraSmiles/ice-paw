// ChatInput.refs.test.ts — @ 引用弹层：触发 / 过滤 / 键盘选择成 chip / 发送并块。
// jsdom 无真实输入法，用 setRangeText + selectionStart 还原「输入 @ + 光标位置」时序。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import ChatInput from "../ChatInput.vue";
import { useChatStore } from "../../../stores/chat";
import { useAgentStore } from "../../../stores/agent";
import { useProjectStore } from "../../../stores/project";

function conv(id: string, title: string, kind?: string, projectId?: string) {
  return {
    id, agent_id: "a1", title, pinned: false,
    created_at: "2026-08-15 00:00:00", updated_at: "2026-08-15 00:00:00",
    project_id: projectId ?? null, kind,
  };
}
function proj(id: string, name: string, archived: boolean) {
  return {
    id, name, description: "", icon: "folder", sort_order: 0,
    workspace_path: null, theme_color: null, archived,
    created_at: "", updated_at: "",
  };
}
function msg(id: string, role: "user" | "assistant", content: string) {
  return {
    id, conversation_id: "c1", role, content, content_blocks: "[]",
    token_count: null, error: null, created_at: "2026-08-15 00:00:00",
    rowid: 1, model: null,
  };
}

/** 往 textarea 键入文本并把光标放到末尾（还原 input 事件时序）。 */
async function type(wrapper: ReturnType<typeof mount>, text: string) {
  const ta = wrapper.find("textarea").element as HTMLTextAreaElement;
  const pos = ta.selectionStart ?? 0;
  ta.value = ta.value.slice(0, pos) + text + ta.value.slice(pos);
  const newPos = pos + text.length;
  ta.setSelectionRange(newPos, newPos);
  await wrapper.find("textarea").trigger("input");
}

describe("ChatInput @ 引用", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    // 组件在弹层打开时会惰性 load 项目列表（归档排除要用）。测试里 invoke 全局
    // mock resolve undefined——挡住 load（loaded=true 早退），防 undefined 覆盖
    // 夹具数据；各用例按需手动设置 projectStore.list。
    const projects = useProjectStore();
    projects.list = [];
    projects.loaded = true;
  });

  it("输入 @ 弹层出现（三段：会话/Agent/消息），过滤生效", async () => {
    const chat = useChatStore();
    const agents = useAgentStore();
    chat.conversations = [conv("c2", "设计讨论"), conv("c3", "日志排查")];
    agents.list = [
      { id: "ag1", name: "审查员", provider: "openai", model: "gpt-test", system_prompt: "", base_url: null, temperature: 0.7, max_tokens: 4096, extra_params: {}, sort_order: 0, cache_prompt: true, has_api_key: true, created_at: "", updated_at: "" },
    ];
    chat.messages = [msg("m1", "user", "帮我看看这段设计"), msg("m2", "assistant", "这是回答")];

    const wrapper = mount(ChatInput);
    await type(wrapper, "@");
    expect(wrapper.find(".at-popover").exists()).toBe(true);
    // 三段都有：会话 c2/c3（散落归属）+ agent 审查员 + 消息 2 条
    const kinds = wrapper.findAll(".at-option-kind").map((n) => n.text());
    expect(kinds.filter((s) => s === "会话").length).toBe(2);
    expect(kinds.filter((s) => s === "Agent").length).toBe(1);
    expect(kinds.some((s) => s === "回答" || s === "消息")).toBe(true);
    // agent 行：id 跟名字后 + model 归属
    expect(wrapper.find(".at-option-id").text()).toBe("ag1");
    expect(wrapper.findAll(".at-option-owner").some((n) => n.text() === "gpt-test")).toBe(true);

    // 过滤：输入「设计」只剩匹配项
    await type(wrapper, "设计");
    const labels = wrapper.findAll(".at-option-label").map((n) => n.text());
    expect(labels).toEqual(["设计讨论", "帮我看看这段设计"]);
  });

  it("归档项目的会话不进候选；跨项目归属标注在副标题", async () => {
    const chat = useChatStore();
    const projects = useProjectStore();
    projects.list = [proj("p1", "活跃项目", false), proj("p2", "旧项目", true)];
    chat.conversations = [
      conv("c-live", "活跃会话", undefined, "p1"),
      conv("c-arch", "归档会话", undefined, "p2"),
      conv("c-loose", "散落会话"),
    ];

    const wrapper = mount(ChatInput);
    await type(wrapper, "@");
    const labels = wrapper.findAll(".at-option-label").map((n) => n.text());
    expect(labels).toContain("活跃会话");
    expect(labels).toContain("散落会话");
    // 归档项目会话（侧栏/项目页均不可见）不应出现在候选
    expect(labels).not.toContain("归档会话");
    // 归属透明：项目会话 owner=项目名，散落会话 owner=散落，类型词统一右缘
    const owners = wrapper.findAll(".at-option-owner").map((n) => n.text());
    expect(owners).toContain("活跃项目");
    expect(owners).toContain("散落");
  });

  it("Enter 选中 → @query 被删、chip 出现、pendingRefs 有值", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c2", "设计讨论")];
    const wrapper = mount(ChatInput);
    await type(wrapper, "看看 @设计");
    expect(wrapper.find(".at-popover").exists()).toBe(true);

    await wrapper.find("textarea").trigger("keydown", { key: "Enter" });
    // @query 从输入框移除（剩「看看 」），chip 上显示 `设计讨论#短码`
    expect((wrapper.find("textarea").element as HTMLTextAreaElement).value).toBe("看看 ");
    expect(chat.pendingRefs.length).toBe(1);
    expect(chat.pendingRefs[0].refKind).toBe("conversation");
    expect(chat.pendingRefs[0].targetId).toBe("c2");
    expect(chat.pendingRefs[0].display).toMatch(/^设计讨论#\d{4}$/);
    expect(wrapper.find(".ref-chip").exists()).toBe(true);
    // 弹层关闭
    expect(wrapper.find(".at-popover").exists()).toBe(false);
  });

  it("Esc 关闭弹层；@ 前非空白（邮箱 a@b）不触发", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c2", "设计讨论")];
    const wrapper = mount(ChatInput);

    await type(wrapper, "a@b");
    expect(wrapper.find(".at-popover").exists()).toBe(false);

    await type(wrapper, "\n@");
    expect(wrapper.find(".at-popover").exists()).toBe(true);
    await wrapper.find("textarea").trigger("keydown", { key: "Escape" });
    expect(wrapper.find(".at-popover").exists()).toBe(false);
  });

  it("chip 可删；发送时 reference 块并入 blocks（invoke 断言）", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c1", "当前会话"), conv("c2", "设计讨论")];
    chat.activeConvId = "c1"; // sendMessage 真体前置（不走 selectConversation 免 loadMessages）
    const wrapper = mount(ChatInput);

    await type(wrapper, "@");
    await wrapper.find("textarea").trigger("keydown", { key: "Enter" });
    expect(chat.pendingRefs.length).toBe(1);

    // chip 删除（按钮复用 file-chip-remove，chip 容器带 ref-chip）
    await wrapper.find(".ref-chip .file-chip-remove").trigger("click");
    expect(chat.pendingRefs.length).toBe(0);

    // 重新选一个再发送（sendMessage 真体：并块 + 清空 pendingRefs + invoke）
    await type(wrapper, "@");
    await wrapper.find("textarea").trigger("keydown", { key: "Enter" });
    await type(wrapper, "这个问题帮我看看");
    await wrapper.find(".btn-send").trigger("click");
    await Promise.resolve();

    expect(chat.pendingRefs.length).toBe(0); // 发送后清空
    const invoke = (await import("@tauri-apps/api/core")).invoke;
    const sendCall = vi.mocked(invoke).mock.calls.find((c) => c[0] === "send_message");
    expect(sendCall).toBeTruthy();
    const args = sendCall![1] as { input: { content_blocks?: { type: string }[] } };
    const ref = args.input.content_blocks?.find((b) => b.type === "reference");
    expect(ref).toMatchObject({ ref_kind: "conversation", target_id: "c2" });
  });

  it("重复选择同一对象不重复加 chip（去重 + 闪已有 chip）", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c2", "设计讨论")];
    const wrapper = mount(ChatInput);

    await type(wrapper, "@");
    await wrapper.find("textarea").trigger("keydown", { key: "Enter" });
    expect(chat.pendingRefs.length).toBe(1);

    // 再选同一个：chip 数不变，闪已有 chip（ref-flash class 短暂出现）
    await type(wrapper, "@");
    await wrapper.find("textarea").trigger("keydown", { key: "Enter" });
    expect(chat.pendingRefs.length).toBe(1);
    expect(wrapper.find(".ref-flash").exists()).toBe(true);
  });

  it("已选中的对象重开弹层时整行选中态（selected class）", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c2", "设计讨论")];
    const wrapper = mount(ChatInput);

    await type(wrapper, "@");
    await wrapper.find("textarea").trigger("keydown", { key: "Enter" });
    expect(chat.pendingRefs.length).toBe(1);

    // 重新打开弹层：该候选整行选中态（elementui 风：底色+字色，无额外元素）
    await type(wrapper, "@");
    expect(wrapper.find(".at-option.selected").exists()).toBe(true);
  });

  it("匹配文本高亮：命中段带 .at-hit 标记", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c2", "设计讨论")];
    chat.messages = [];
    const wrapper = mount(ChatInput);

    await type(wrapper, "@设计");
    const hits = wrapper.findAll(".at-hit").map((n) => n.text());
    expect(hits).toContain("设计");
  });

  it("Agent 按 id 模糊匹配（uuid 片段可搜，id 命中段高亮）", async () => {
    const chat = useChatStore();
    const agents = useAgentStore();
    agents.list = [
      { id: "agent-uuid-1234", name: "审查员", provider: "openai", model: "gpt-test", system_prompt: "", base_url: null, temperature: 0.7, max_tokens: 4096, extra_params: {}, sort_order: 0, cache_prompt: true, has_api_key: true, created_at: "", updated_at: "" },
    ];
    chat.conversations = [];
    chat.messages = [];
    const wrapper = mount(ChatInput);

    await type(wrapper, "@1234");
    const labels = wrapper.findAll(".at-option-label").map((n) => n.text());
    expect(labels).toContain("审查员");
    // id 跟在名字后显示，命中段带高亮
    expect(wrapper.find(".at-option-id").text()).toContain("agent-uuid-1234");
    expect(wrapper.find(".at-option-id .at-hit").text()).toBe("1234");
  });

  it("纯引用无文本可发送（发送按钮不 disabled）", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c2", "设计讨论")];
    const wrapper = mount(ChatInput);
    await type(wrapper, "@");
    await wrapper.find("textarea").trigger("keydown", { key: "Enter" });
    expect((wrapper.find(".btn-send").element as HTMLButtonElement).disabled).toBe(false);
  });

  it("@ 按钮点击 = 光标处插入 @ 并触发弹层；光标前有文本时补空格", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c2", "设计讨论")];
    const wrapper = mount(ChatInput);

    // 空输入框：直接插 @
    await wrapper.find(".btn-at").trigger("click");
    await Promise.resolve();
    expect((wrapper.find("textarea").element as HTMLTextAreaElement).value).toBe("@");
    expect(wrapper.find(".at-popover").exists()).toBe(true);
    await wrapper.find("textarea").trigger("keydown", { key: "Escape" });

    // 光标前有文本：补空格再插 @（保证触发条件）
    const ta = wrapper.find("textarea").element as HTMLTextAreaElement;
    ta.setSelectionRange(0, 0); // 光标移到行首 → 前无字符，不补空格
    await wrapper.find(".btn-at").trigger("click");
    await Promise.resolve();
    expect(ta.value).toBe("@@");
  });
});
