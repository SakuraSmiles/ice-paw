// ChatInput.channel.test.ts — 频道 v1 的 @ 点名通路锁定：
// 候选 = 频道成员（统筹者 owner 标注）、选择 = 文本保留 `@名字 `（无 chip 无
// reference 块）、send 时 collectMentions 从文本解析 agent_id 列表（与后端
// parse_agent_mentions 同构：最长前缀 / email 防御 / 重名歧义跳过 / 去重保序）、
// 归档频道只读（textarea disabled + 发送拦截）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import ChatInput from "../ChatInput.vue";
import { useChatStore } from "../../../stores/chat";
import { useProjectStore } from "../../../stores/project";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

function channelConv(archived = false) {
  return {
    id: "ch1", agent_id: "ag1", title: "项目频道", pinned: false,
    created_at: "2026-09-10 00:00:00", updated_at: "2026-09-10 00:00:00",
    project_id: "p1", kind: "channel",
    archived_at: archived ? "2026-09-10 09:00:00" : null,
  };
}

function members(list: { id: string; name: string; role?: string }[]) {
  return list.map((m) => ({ agent_id: m.id, name: m.name, role: m.role ?? "member" }));
}

async function setup(archived = false) {
  const chat = useChatStore();
  chat.conversations = [channelConv(archived)];
  chat.activeConvId = "ch1";
  return chat;
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

function lastSendInput(): { mentions?: string[] } | null {
  const call = mockInvoke.mock.calls.filter((c) => c[0] === "send_message").pop();
  return call ? (call[1] as { input: { mentions?: string[] } }).input : null;
}

describe("ChatInput 频道 @ 点名", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue(undefined as never);
    const projects = useProjectStore();
    projects.list = [];
    projects.loaded = true;
  });

  it("候选 = 频道成员（kindText「成员」；统筹者 owner 标注），不出现会话/消息段", async () => {
    const chat = await setup();
    chat.channelView = { channel: null, members: members([{ id: "ag1", name: "写手", role: "coordinator" }, { id: "ag2", name: "审校" }]), coordinator_agent_id: "ag1", coordinator_appointed: false };

    const wrapper = mount(ChatInput);
    await type(wrapper, "@");
    expect(wrapper.find(".at-popover").exists()).toBe(true);
    const kinds = wrapper.findAll(".at-option-kind").map((n) => n.text());
    expect(kinds.every((s) => s === "成员")).toBe(true); // 频道分支无三段引用
    const owners = wrapper.findAll(".at-option-owner").map((n) => n.text());
    expect(owners).toContain("统筹者");
    const labels = wrapper.findAll(".at-option-label").map((n) => n.text());
    expect(labels).toEqual(["写手", "审校"]);

    // 过滤：输入「审」只剩审校
    await type(wrapper, "审");
    expect(wrapper.findAll(".at-option-label").map((n) => n.text())).toEqual(["审校"]);
  });

  it("选中成员 = 文本替换为 `@名字 `（无 chip、无 pendingRefs）", async () => {
    const chat = await setup();
    chat.channelView = { channel: null, members: members([{ id: "ag1", name: "写手" }]), coordinator_agent_id: null, coordinator_appointed: false };

    const wrapper = mount(ChatInput);
    await type(wrapper, "请 @写");
    await wrapper.findAll(".at-option")[0].trigger("click");
    // 文本保留点名可见（与 1v1 引用「删文本成 chip」语义相反）
    expect((wrapper.find("textarea").element as HTMLTextAreaElement).value).toBe("请 @写手 ");
    expect(wrapper.find(".at-popover").exists()).toBe(false);
    expect(chat.pendingRefs.length).toBe(0); // 无 chip
    expect(wrapper.find(".ref-chip").exists()).toBe(false);
  });

  it("send 解析 mentions：点名命中 + 去重保序（invoke 断言）", async () => {
    const chat = await setup();
    chat.channelView = { channel: null, members: members([{ id: "ag1", name: "写手" }, { id: "ag2", name: "审校" }]), coordinator_agent_id: null, coordinator_appointed: false };

    const wrapper = mount(ChatInput);
    await type(wrapper, "@审校 你先看，@写手 再改，@审校 不重复");
    await wrapper.find(".btn-send").trigger("click");
    await Promise.resolve();

    const input = lastSendInput();
    expect(input).toBeTruthy();
    expect(input!.mentions).toEqual(["ag2", "ag1"]); // 去重 + 首现序
  });

  it("send 解析护栏语义：最长前缀胜出 / email 防御 / 重名歧义跳过", async () => {
    const chat = await setup();
    chat.channelView = {
      channel: null,
      members: members([
        { id: "ag1", name: "审校" }, { id: "ag2", name: "审校长" },
        { id: "ag3", name: "影子" }, { id: "ag4", name: "影子" },
      ]),
      coordinator_agent_id: null, coordinator_appointed: false,
    };

    const wrapper = mount(ChatInput);
    await type(wrapper, "@审校长 请把关；邮箱 someone@shadow.com 不是点名；@影子 重名不猜");
    await wrapper.find(".btn-send").trigger("click");
    await Promise.resolve();

    const input = lastSendInput();
    expect(input!.mentions).toEqual(["ag2"]); // 最长前缀（审校长）+ email 不触发 + 重名跳过
  });

  it("1v1 会话发送不传 mentions（后端忽略，前端也不解析）", async () => {
    const chat = useChatStore();
    chat.conversations = [{ id: "c1", agent_id: "ag1", title: "普通会话", pinned: false, created_at: "", updated_at: "", project_id: null, kind: "chat" }];
    chat.activeConvId = "c1";

    const wrapper = mount(ChatInput);
    await type(wrapper, "@写手 你在吗");
    await wrapper.find(".btn-send").trigger("click");
    await Promise.resolve();

    const input = lastSendInput();
    expect(input).toBeTruthy();
    expect(input!.mentions).toBeUndefined();
  });

  it("归档频道只读：textarea disabled + 归档 placeholder + hint，发送拦截", async () => {
    await setup(true); // archived
    const wrapper = mount(ChatInput);

    const ta = wrapper.find("textarea").element as HTMLTextAreaElement;
    expect(ta.disabled).toBe(true);
    expect(ta.placeholder).toContain("频道已归档");
    expect(wrapper.find(".input-hint").text()).toBe("频道已归档");
    expect((wrapper.find(".btn-send").element as HTMLButtonElement).disabled).toBe(true);
    expect((wrapper.find(".btn-at").element as HTMLButtonElement).disabled).toBe(true);

    // disabled 按钮不触发 send（无 send_message 调用）
    await wrapper.find(".btn-send").trigger("click");
    expect(mockInvoke.mock.calls.some((c) => c[0] === "send_message")).toBe(false);
  });
});
