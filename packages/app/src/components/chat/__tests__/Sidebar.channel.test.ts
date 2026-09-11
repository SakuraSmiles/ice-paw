// Sidebar.channel.test.ts — 侧栏频道区块（§6.1）形态锁定：
// ① 项目 scope 活频道一行（标题 + 「统筹 · 名」meta；无统筹者诚实「待选举」）
//    ⚠️ 统筹者权威源 = project.agents role='coordinator'（2026-09-11 生产实案：
//    旧实现读 conv.agent_id——ensure 投影未选举时回落最早成员 → 选举前谎报名字）
// ② 频道不进普通会话列表（isUserChat 排除，恒由本区块独占渲染）
// ③ 未开启 → 「开启频道」懒建入口（ensure_channel → 选中新建频道）；失败行内 caption
// ④ 散落 scope = 归档频道只读行（「已归档」标；原项目删除 FK SET NULL 转散落）
// ⑤ 搜索框移除回归锁（2026-09-10 拍板，随频道改版整体移除）
// ⑥ 统筹位落定事件（channel_coordinator）→ 刷新项目列表，「待选举」回正
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Sidebar from "../Sidebar.vue";
import { useChatStore } from "../../../stores/chat";
import { useAgentStore } from "../../../stores/agent";
import { useProjectStore } from "../../../stores/project";
import type { Conversation } from "../../../types";

const mockInvoke = vi.mocked(invoke);
const push = vi.fn();

vi.mock("vue-router", () => ({
  useRouter: () => ({
    push,
    currentRoute: { value: { name: "Home", path: "/", fullPath: "/" } },
  }),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: () => Promise.resolve() }),
}));

function conv(id: string, over: Partial<Conversation> = {}): Conversation {
  return {
    id, agent_id: "ag1", title: id, pinned: false,
    created_at: "2026-09-10 10:00:00", updated_at: "2026-09-10 10:00:00",
    project_id: null, kind: "chat",
    ...over,
  } as Conversation;
}

const agentRows = [
  { id: "ag1", name: "写手", provider: "zhipu", model: "glm-5.3", system_prompt: "", base_url: null, temperature: 0.7, max_tokens: 4096, extra_params: {}, sort_order: 0, cache_prompt: true, has_api_key: true, created_at: "", updated_at: "", avatar: null },
  { id: "ag9", name: "前统筹", provider: "zhipu", model: "glm-5.3", system_prompt: "", base_url: null, temperature: 0.7, max_tokens: 4096, extra_params: {}, sort_order: 0, cache_prompt: true, has_api_key: true, created_at: "", updated_at: "", avatar: null },
];

/** 项目 fixture：agents 成员表（统筹者显示的权威源） */
function proj(members: Array<{ agent_id: string; role: string }>) {
  return {
    id: "p1", name: "测试项目", description: "", icon: "folder", workspace_path: null,
    created_at: "2026-09-10 10:00:00", updated_at: "2026-09-10 10:00:00",
    agents: members.map((m) => ({ project_id: "p1", ...m, joined_at: "2026-09-10 09:00:00" })),
  } as never;
}

async function mountSidebar() {
  const w = mount(Sidebar);
  await flushPromises();
  // onMounted 的 agent.load() 会把列表整替为 []（invoke 统一返回 []）——mount 后重播
  useAgentStore().list = agentRows as never;
  return w;
}

describe("Sidebar 频道区块", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue([] as never);
    push.mockReset();
    useAgentStore().list = agentRows as never;
  });

  it("项目 scope 活频道：一行 + 统筹者名 meta（权威源 = project.agents coordinator）；不进普通会话列表", async () => {
    const w = await mountSidebar();
    const chat = useChatStore();
    const project = useProjectStore();
    chat.conversations = [
      conv("普通会话", { title: "普通会话", project_id: "p1" }),
      conv("ch-live", { title: "发布频道", kind: "channel", project_id: "p1", agent_id: "ag1" }),
    ];
    useProjectStore().list = [proj([{ agent_id: "ag1", role: "coordinator" }, { agent_id: "ag9", role: "member" }])];
    project.activeProjectId = "p1"; // mount 后设（避开 onMounted load 的时序）
    await flushPromises();

    const item = w.find(".channel-block .channel-item");
    expect(item.exists()).toBe(true);
    expect(item.find(".conv-name").text()).toBe("发布频道");
    expect(item.find(".conv-agent-name").text()).toBe("统筹 · 写手");
    // 有统筹者才出头像（未选举隐藏，防 fallback 成员头像配「待选举」的混淆）
    expect(item.find(".conv-agent-avatar").exists()).toBe(true);
    // 频道独占渲染：普通会话列表（.conv-list）里只有普通会话
    const listNames = w.findAll(".conv-list .conv-item .conv-name").map((n) => n.text());
    expect(listNames).toEqual(["普通会话"]);
  });

  it("统筹位空缺 → 诚实「待选举」——即使 conv.agent_id 投影指向存在的成员（2026-09-11 生产实案回归锁）", async () => {
    // 实案形态：ensure_channel 投影回落 joined_at 最早成员（ag1 写手存在），
    // 但项目无 role='coordinator' 成员（未选举）——旧实现读 conv.agent_id
    // 会谎报「统筹 · 写手」
    const w = await mountSidebar();
    const chat = useChatStore();
    const project = useProjectStore();
    chat.conversations = [conv("ch-live", { kind: "channel", project_id: "p1", agent_id: "ag1" })];
    useProjectStore().list = [proj([{ agent_id: "ag1", role: "member" }, { agent_id: "ag9", role: "member" }])];
    project.activeProjectId = "p1";
    await flushPromises();
    expect(w.find(".channel-block .conv-agent-name").text()).toBe("统筹 · 待选举");
    expect(w.find(".channel-block .conv-agent-avatar").exists()).toBe(false);
  });

  it("统筹位落定事件 → 刷新项目列表，「待选举」回正为统筹者名", async () => {
    // 初始无统筹者（项目成员全 member）
    const w = await mountSidebar();
    const chat = useChatStore();
    const project = useProjectStore();
    chat.conversations = [conv("ch-live", { kind: "channel", project_id: "p1", agent_id: "ag1" })];
    useProjectStore().list = [proj([{ agent_id: "ag1", role: "member" }])];
    project.activeProjectId = "p1";
    await flushPromises();
    expect(w.find(".channel-block .conv-agent-name").text()).toBe("统筹 · 待选举");

    // 选举完成：channel_coordinator 事件广播；刷新后的项目带 coordinator
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_projects") {
        return [proj([{ agent_id: "ag1", role: "member" }, { agent_id: "ag9", role: "coordinator" }])];
      }
      return [];
    });
    mockInvoke.mockClear(); // 排除 mount 期 project.load 的既有调用
    const registrations = vi.mocked(listen).mock.calls.filter(([ev]) => ev === "session:event-appended");
    const onEvent = registrations[registrations.length - 1]![1];
    onEvent({ payload: { conversation_id: "ch-live", kind: "channel_coordinator" } } as never);
    await flushPromises();

    expect(mockInvoke.mock.calls.some((c) => c[0] === "list_projects")).toBe(true);
    expect(w.find(".channel-block .conv-agent-name").text()).toBe("统筹 · 前统筹");
    // 其它 kind 不触发刷新
    mockInvoke.mockClear();
    onEvent({ payload: { conversation_id: "ch-live", kind: "turn_ended" } } as never);
    await flushPromises();
    expect(mockInvoke.mock.calls.some((c) => c[0] === "list_projects")).toBe(false);
  });

  it("未开启 → 「开启频道」懒建：ensure_channel → 选中返回的频道", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "ensure_channel") {
        return {
          channel: conv("ch-new", { title: "P · 频道", kind: "channel", project_id: "p1" }),
          members: [], coordinator_agent_id: null, coordinator_appointed: false,
        };
      }
      return [];
    });
    const w = await mountSidebar();
    const chat = useChatStore();
    const project = useProjectStore();
    chat.conversations = [conv("普通会话", { title: "普通会话" })];
    project.activeProjectId = "p1";
    await flushPromises();

    const openBtn = w.find(".channel-open");
    expect(openBtn.exists()).toBe(true);
    expect(openBtn.find(".conv-name").text()).toBe("开启频道");

    await openBtn.trigger("click");
    await flushPromises();
    expect(mockInvoke.mock.calls.some((c) => c[0] === "ensure_channel")).toBe(true);
    expect(useChatStore().activeConvId).toBe("ch-new");
  });

  it("开启失败 → 行内 caption（后端三段式文案原样透出）", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "ensure_channel") throw new Error("项目还没有成员：先在项目页添加成员再开启频道");
      return [];
    });
    const w = await mountSidebar();
    const chat = useChatStore();
    const project = useProjectStore();
    chat.conversations = [];
    project.activeProjectId = "p1";
    await flushPromises();

    await w.find(".channel-open").trigger("click");
    await flushPromises();
    expect(w.find(".channel-error").text()).toContain("项目还没有成员");
    // 失败后可重试（ensuring 复位、入口仍在）
    expect(w.find(".channel-open").exists()).toBe(true);
  });

  it("散落 scope：归档频道只读行带「已归档」标；点击选中", async () => {
    const w = await mountSidebar();
    const chat = useChatStore();
    chat.conversations = [
      conv("ch-old", { title: "已删项目的频道", kind: "channel", project_id: null, archived_at: "2026-09-10 09:00:00" } as Partial<Conversation>),
      conv("散落会话", { title: "散落会话" }),
    ];
    await flushPromises();

    const archived = w.findAll(".sidebar-top .channel-item");
    expect(archived.length).toBe(1);
    expect(archived[0].find(".conv-name").text()).toBe("已删项目的频道");
    expect(archived[0].find(".channel-archived-tag").text()).toBe("已归档");

    await archived[0].trigger("click");
    await flushPromises();
    expect(useChatStore().activeConvId).toBe("ch-old");
  });

  it("搜索框移除回归锁（展开态 sidebar-top 无搜索框）", async () => {
    const w = await mountSidebar();
    expect(w.find(".sidebar-search").exists()).toBe(false);
  });
});
