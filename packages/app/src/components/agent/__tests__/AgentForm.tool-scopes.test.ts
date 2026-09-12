// AgentForm.tool-scopes.test.ts — 工具集范围区块行为锁（2026-09-12 批次）：
// ① 编辑态渲染/默认全开（无 scopes → 「全部工具」胶囊，保存零旋钮调用——不改不写）；
// ② 自定义勾选 → 保存走旋钮通道 setToolScopes（不进 update 出生证 payload）；
// ③ 「全部工具」清空草稿 → 保存摘除（null）；④ 已保存死条目（已删 server /
// 裸工具名）保留进草稿防丢、chip 可摘；⑤ 唯一权威（二批）：旧 enabled_tools
// 白名单打开即并入草稿（平台元工具过滤）、保存工具面改动时随批摘除（次序先
// scopes 后摘白名单）；不动工具面零强制迁移。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import type { Agent } from "../../../types";

// 顶层 Tauri 插件 import 须先 mock（同 AgentForm.avatar.test.ts）
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));

const providersListMock = vi.fn();
const profilesListMock = vi.fn();
const updateMock = vi.fn();
const setToolScopesMock = vi.fn();
const setEnabledToolsMock = vi.fn();
const builtinToolsMock = vi.fn();
const mcpListMock = vi.fn();

vi.mock("../../../api/bridge", () => ({
  bridge: {
    providers: {
      list: (...a: unknown[]) => providersListMock(...a),
      testConnection: vi.fn(),
    },
    preferences: { get: async () => ({}) },
    agents: {
      create: vi.fn(),
      update: (...a: unknown[]) => updateMock(...a),
      rotateKey: vi.fn(),
      setToolScopes: (...a: unknown[]) => setToolScopesMock(...a),
      setEnabledTools: (...a: unknown[]) => setEnabledToolsMock(...a),
      list: async () => [],
    },
    modelProfiles: { list: (...a: unknown[]) => profilesListMock(...a) },
    mcp: {
      listBuiltinTools: (...a: unknown[]) => builtinToolsMock(...a),
      list: (...a: unknown[]) => mcpListMock(...a),
    },
  },
}));

function editAgent(overrides?: Partial<Agent>): Agent {
  return {
    id: "ag-1",
    name: "助手",
    provider: "openai",
    model: "gpt-4o",
    system_prompt: "",
    base_url: null,
    temperature: 0.7,
    max_tokens: 16384,
    extra_params: {},
    sort_order: 0,
    cache_prompt: true,
    workspace_path: null,
    config_from_file: false,
    created_at: "2026-09-12 00:00:00",
    updated_at: "2026-09-12 00:00:00",
    has_api_key: true,
    model_profile_id: "mp-1",
    ...overrides,
  } as Agent;
}

/** 内置工具清单（组计数数据源；两组足够驱动渲染） */
const BUILTINS = [
  { name: "read_file", description: "read", group: "files" },
  { name: "write_file", description: "write", group: "files" },
  { name: "search_kb", description: "kb", group: "kb" },
];

/** 外部 server 快照（scopeOptions 只消费 id/name） */
const UE5_SERVER = { id: "ue5-mcp", name: "UE5 MCP", enabled: true, status: "running" };

const wrappers: VueWrapper[] = [];

async function mountForm(agent: Agent | null = null) {
  providersListMock.mockResolvedValue([
    { name: "openai", protocol: "openai", default_url: "https://api.openai.com", alt_urls: [], label: "OpenAI", note: null, requires_key: true, requires_base_url: false, key_url: null, hidden: false, models: ["gpt-4o"] },
  ]);
  profilesListMock.mockResolvedValue([
    {
      id: "mp-1", alias: "OpenAI 主力", provider: "openai", model: "gpt-4o",
      base_url: null, sort_order: 0,
      created_at: "2026-09-08 00:00:00", updated_at: "2026-09-08 00:00:00",
      has_api_key: true, last_health: null, last_health_detail: null, last_health_at: null,
    },
  ]);
  builtinToolsMock.mockResolvedValue(BUILTINS);
  mcpListMock.mockResolvedValue([UE5_SERVER]);
  updateMock.mockResolvedValue(editAgent());
  setToolScopesMock.mockResolvedValue({});
  setEnabledToolsMock.mockResolvedValue({});
  const { default: AgentForm } = await import("../AgentForm.vue");
  const w = mount(AgentForm, { props: { agent }, attachTo: document.body });
  wrappers.push(w);
  await flushPromises();
  return w;
}

function saveBtn(w: VueWrapper) {
  return w.find(".section-actions .btn-primary");
}

describe("AgentForm 工具集范围区块", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    for (const w of wrappers) w.unmount();
    wrappers.length = 0;
    document.body.innerHTML = "";
  });

  it("编辑态默认全开：无 scopes → 「全部工具」胶囊激活、无选项区；保存零旋钮调用（不改不写）", async () => {
    const w = await mountForm(editAgent());
    const pills = w.findAll(".scope-pill");
    expect(pills).toHaveLength(2);
    expect(pills[0].text()).toBe("全部工具");
    expect(pills[0].classes()).toContain("active");
    expect(w.find(".scope-options").exists()).toBe(false);

    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(setToolScopesMock).not.toHaveBeenCalled();
  });

  it("选项数据源：内置组（带计数）+ 外部 server 每台一项", async () => {
    const w = await mountForm(editAgent());
    // 默认全开 → 先切自定义
    await w.findAll(".scope-pill")[1].trigger("click");
    const opts = w.findAll(".scope-option");
    const labels = opts.map((o) => o.find(".opt-label").text());
    expect(labels).toContain("文件与命令");
    expect(labels).toContain("知识库");
    expect(labels).toContain("UE5 MCP");
    // 组计数来自 listBuiltinTools 聚合（files 2 件）
    const filesOpt = opts.find((o) => o.find(".opt-label").text() === "文件与命令")!;
    expect(filesOpt.find(".opt-note").text()).toBe("2 件");
    // server 项 value 携带 server: 前缀
    expect((filesOpt.find("input").element as HTMLInputElement).value).toBe("group:files");
    const ue5Opt = opts.find((o) => o.find(".opt-label").text() === "UE5 MCP")!;
    expect((ue5Opt.find("input").element as HTMLInputElement).value).toBe("server:ue5-mcp");
  });

  it("自定义勾选 → 保存走旋钮通道 setToolScopes，且不进 update 出生证 payload", async () => {
    const w = await mountForm(editAgent());
    await w.findAll(".scope-pill")[1].trigger("click");
    const opts = w.findAll(".scope-option");
    const filesOpt = opts.find((o) => o.find(".opt-label").text() === "文件与命令")!;
    await filesOpt.find("input").setValue(true);
    const ue5Opt = opts.find((o) => o.find(".opt-label").text() === "UE5 MCP")!;
    await ue5Opt.find("input").setValue(true);

    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(setToolScopesMock).toHaveBeenCalledTimes(1);
    expect(setToolScopesMock).toHaveBeenCalledWith("ag-1", ["group:files", "server:ue5-mcp"]);
    // 出生证通道不含工具面字段
    const payload = updateMock.mock.calls[0][0];
    expect(payload.tool_scopes).toBeUndefined();
    expect(payload.enabled_tools).toBeUndefined();
  });

  it("既有 scopes 预填自定义态；不改保存零调用（草稿未动）", async () => {
    const w = await mountForm(editAgent({ tool_scopes: ["group:kb"] }));
    expect(w.findAll(".scope-pill")[1].classes()).toContain("active");
    const kbOpt = w.findAll(".scope-option").find((o) => o.find(".opt-label").text() === "知识库")!;
    expect((kbOpt.find("input").element as HTMLInputElement).checked).toBe(true);

    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(setToolScopesMock).not.toHaveBeenCalled();
  });

  it("「全部工具」清空草稿 → 保存摘除（null 恢复全开）", async () => {
    const w = await mountForm(editAgent({ tool_scopes: ["group:kb"] }));
    await w.findAll(".scope-pill")[0].trigger("click");
    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(setToolScopesMock).toHaveBeenCalledWith("ag-1", null);
  });

  it("死条目（已删 server / 裸工具名）保留进草稿防丢，chip 摘除后随批提交", async () => {
    const w = await mountForm(editAgent({ tool_scopes: ["group:files", "server:gone", "run_command"] }));
    // 选项外的两条以 chip 呈现（可感知、可单独摘）
    const chips = w.findAll(".scope-extra-chip");
    expect(chips.map((c) => c.find(".chip-text").text())).toEqual(["server:gone", "run_command"]);

    // 摘掉死 server 条目，保留其余 → 保存提交剩余集合
    await chips[0].find(".chip-x").trigger("click");
    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(setToolScopesMock).toHaveBeenCalledWith("ag-1", ["group:files", "run_command"]);
  });

  it("旧白名单 → 接管声明 + 非平台条目并入草稿 chip；不动工具面保存零写入（零强制迁移）", async () => {
    const w = await mountForm(editAgent({
      enabled_tools: ["read_file", "propose_config_change"],
    }));
    // 接管声明取代旧「取交集」并列提示（并列即歧义，2026-09-12 二批拍板）
    expect(w.text()).toContain("检测到旧版工具白名单（2 项）");
    expect(w.text()).not.toContain("取交集");
    // 非平台条目并入草稿 → 自定义态 + chip；平台元工具恒可见无需表达（过滤）
    expect(w.findAll(".scope-pill")[1].classes()).toContain("active");
    expect(w.findAll(".scope-extra-chip").map((c) => c.find(".chip-text").text()))
      .toEqual(["read_file"]);

    // 并入条目已在草稿基线内（非 dirty）→ 保存零旋钮调用，白名单原样保留
    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(setToolScopesMock).not.toHaveBeenCalled();
    expect(setEnabledToolsMock).not.toHaveBeenCalled();
  });

  it("保存工具面改动 → scopes 落库后随批摘除旧白名单（次序锁：先写后摘）", async () => {
    const w = await mountForm(editAgent({ enabled_tools: ["read_file"] }));
    // 自定义态（chip read_file 已并入草稿基线）；再勾知识库组 → dirty
    const kbOpt = w.findAll(".scope-option").find((o) => o.find(".opt-label").text() === "知识库")!;
    await kbOpt.find("input").setValue(true);

    await saveBtn(w).trigger("click");
    await flushPromises();
    // 白名单条目随批进 scopes（勾选顺序 = 基线在前、新勾在后）
    expect(setToolScopesMock).toHaveBeenCalledWith("ag-1", ["read_file", "group:kb"]);
    expect(setEnabledToolsMock).toHaveBeenCalledWith("ag-1", null);
    // 先写 scopes 再摘白名单——中间态两侧并设 = 交集，只会更窄不会放宽
    expect(setEnabledToolsMock.mock.invocationCallOrder[0])
      .toBeGreaterThan(setToolScopesMock.mock.invocationCallOrder[0]);
  });

  it("旧白名单 + 切「全部工具」→ 保存双双摘除（真全开）", async () => {
    const w = await mountForm(editAgent({ enabled_tools: ["read_file"] }));
    await w.findAll(".scope-pill")[0].trigger("click");
    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(setToolScopesMock).toHaveBeenCalledWith("ag-1", null);
    expect(setEnabledToolsMock).toHaveBeenCalledWith("ag-1", null);
  });

  it("新建态不渲染工具区块（创建后默认全开，收窄进编辑页）", async () => {
    const w = await mountForm(null);
    expect(w.find(".scope-block").exists()).toBe(false);
  });
});
