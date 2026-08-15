// AgentForm.providers.test.ts — 分组模型选择器（Provider+模型合并）行为锁：
// 目录分组来自 bridge.providers.list（hidden 条目不进下拉）、点条目推导
// provider+model、Ollama 组内拉取条目（免 Key 拉到再选）、测试连接成功
// 绿字+拉取模型并入所属组+走通地址回填固化、失败红字、编辑态存量模型
// 兜底条目（可见组插入 / hidden 旧入口合成组）、切厂商清旧端点地址。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import type { Agent, ProviderInfo } from "../../../types";

// AgentForm 顶层 import 了两个 Tauri 插件（选目录/文件管理器），全局 setup 未覆盖，须先 mock
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));

const providersListMock = vi.fn();
const testConnectionMock = vi.fn();
const createMock = vi.fn();

// useProviders 与 AgentForm 各自从不同相对路径 import 同一 bridge 模块——一份 mock 全覆盖
vi.mock("../../../api/bridge", () => ({
  bridge: {
    providers: {
      list: (...a: unknown[]) => providersListMock(...a),
      testConnection: (...a: unknown[]) => testConnectionMock(...a),
    },
    preferences: { get: async () => ({}) },
    agents: {
      create: (...a: unknown[]) => createMock(...a),
      update: vi.fn().mockResolvedValue({}),
      rotateKey: vi.fn().mockResolvedValue(undefined),
      list: async () => [],
    },
  },
}));

const GLM_CODING_URL = "https://open.bigmodel.cn/api/coding/paas/v4";

// 后端注册表镜像（子集足够驱动表单分支；含 hidden 条目验证下拉过滤与兜底合成组）
const PROVIDERS: ProviderInfo[] = [
  { name: "openai", protocol: "openai", default_url: "https://api.openai.com", alt_urls: [], label: "OpenAI", note: null, requires_key: true, requires_base_url: false, hidden: false, models: ["gpt-4o", "gpt-4o-mini"] },
  { name: "glm", protocol: "openai", default_url: "https://open.bigmodel.cn/api/paas/v4", alt_urls: [["Coding 端点", GLM_CODING_URL]], label: "智谱", note: null, requires_key: true, requires_base_url: false, hidden: false, models: ["glm-5-turbo", "glm-4-flash"] },
  { name: "deepseek", protocol: "openai", default_url: "https://api.deepseek.com", alt_urls: [], label: "DeepSeek", note: null, requires_key: true, requires_base_url: false, hidden: false, models: ["deepseek-chat"] },
  { name: "ollama", protocol: "openai", default_url: "http://localhost:11434/v1", alt_urls: [], label: "Ollama 本地", note: "无需 API Key", requires_key: false, requires_base_url: false, hidden: false, models: [] },
  { name: "glm-coding", protocol: "openai", default_url: GLM_CODING_URL, alt_urls: [], label: "智谱 GLM Coding", note: "旧入口", requires_key: true, requires_base_url: false, hidden: true, models: ["glm-5.2"] },
  { name: "custom", protocol: "openai", default_url: "", alt_urls: [], label: "自定义（OpenAI 兼容）", note: "已下线", requires_key: false, requires_base_url: true, hidden: true, models: [] },
];

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
    created_at: "2026-08-15 00:00:00",
    updated_at: "2026-08-15 00:00:00",
    has_api_key: true,
    ...overrides,
  };
}

const wrappers: VueWrapper[] = [];

/** useProviders 是模块级单例缓存——resetModules + 动态 import 保证每个用例拿到干净目录 */
async function mountForm(agent: Agent | null = null) {
  const { default: AgentForm } = await import("../AgentForm.vue");
  const w = mount(AgentForm, { props: { agent }, attachTo: document.body });
  wrappers.push(w);
  await flushPromises();
  return w;
}

/** 表单内文本输入框 DOM 序：[名称, ID, API Key, API URL]（工作区 readonly 另有 class） */
function textInputs(w: VueWrapper) {
  return w.findAll("input.input").filter((i) => !i.element.classList.contains("workspace-input"));
}

function baseUrlInput(w: VueWrapper) {
  return textInputs(w)[3];
}

async function openDropdown(w: VueWrapper) {
  await w.find(".gs-control").trigger("click");
}

/** 点分组里的条目（按文本找，等价真实用户点选） */
async function clickOption(w: VueWrapper, text: string) {
  const el = w.findAll(".gs-option").find((o) => o.text().includes(text));
  expect(el, `下拉应含「${text}」`).toBeTruthy();
  await el!.trigger("click");
  await flushPromises();
}

async function fillBasics(w: VueWrapper, name: string, id: string) {
  const inputs = textInputs(w);
  await inputs[0].setValue(name);
  await inputs[1].setValue(id);
}

async function save(w: VueWrapper) {
  await w.find(".section-actions .btn-primary").trigger("click");
  await flushPromises();
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.resetModules();
  providersListMock.mockResolvedValue(PROVIDERS);
  createMock.mockResolvedValue({});
});

afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount();
});

describe("AgentForm 分组模型选择器", () => {
  it("下拉分组只含可见条目：hidden（自定义/旧入口）不进下拉，组头带品牌图标", async () => {
    const w = await mountForm();
    await openDropdown(w);
    expect(providersListMock).toHaveBeenCalled();
    const labels = w.findAll(".gs-group-label").map((l) => l.text());
    expect(labels).toHaveLength(4); // openai + 智谱 + deepseek + ollama
    expect(labels.join("|")).not.toContain("自定义");
    expect(labels.join("|")).not.toContain("GLM Coding");
    // 每组头一个品牌图标
    expect(w.findAll(".gs-group-label svg.provider-icon")).toHaveLength(4);
    // openai 2 + glm 2 + deepseek 1 + ollama 拉取条目 1
    expect(w.findAll(".gs-option")).toHaveLength(6);
    // 自定义组内联输入框（目录外名字旧入口）已随自定义下线
    expect(w.find(".gs-inline-input").exists()).toBe(false);
  });

  it("点模型条目：provider+model 同时推导，保存走正确归属", async () => {
    const w = await mountForm();
    await openDropdown(w);
    await clickOption(w, "glm-5-turbo");
    await fillBasics(w, "GLM 助手", "glm-helper");
    await textInputs(w)[2].setValue("sk-glm-key");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("glm");
    expect(input.model).toBe("glm-5-turbo");
    // 关闭态回显选中模型名（selector 语义）+ 当前归属厂商图标
    expect(w.find(".gs-value").text()).toBe("glm-5-turbo");
    expect(w.find(".gs-control svg.provider-icon").exists()).toBe(true);
  });

  it("Ollama：组内拉取条目触发探测（免 Key）→ 拉到的模型入组可选中，空 Key 可保存", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 1, models: ["qwen3:8b"], error: null, matched_url: "http://localhost:11434/v1" });
    const w = await mountForm();
    await openDropdown(w);
    await clickOption(w, "拉取已安装模型");
    expect(testConnectionMock).toHaveBeenCalledWith("ollama", undefined, undefined, undefined);
    await flushPromises();
    // 拉取成功后模型并入 ollama 组 → 选中
    await openDropdown(w);
    await clickOption(w, "qwen3:8b");
    await fillBasics(w, "本地助手", "local-helper");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("ollama");
    expect(input.api_key).toBe("");
    expect(input.model).toBe("qwen3:8b");
  });

  it("测试连接成功：绿字 + 拉取模型并入当前厂商组 + 走通地址回填固化（智谱 Coding 备选端点）", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 1, models: ["glm-5.2"], error: null, matched_url: GLM_CODING_URL });
    const w = await mountForm();
    await openDropdown(w);
    await clickOption(w, "glm-5-turbo");
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    expect(testConnectionMock).toHaveBeenCalledWith("glm", undefined, undefined, undefined);
    const ok = w.find(".conn-ok");
    expect(ok.exists()).toBe(true);
    expect(ok.text()).toContain("连接成功，发现 1 个模型");
    // 多端点自动匹配：走通的 Coding 地址回填 API URL（把「这次测通了」固化成「以后都走它」）
    expect((baseUrlInput(w).element as HTMLInputElement).value).toBe(GLM_CODING_URL);
    // 拉取结果并入智谱组（静态目录仍在）
    await openDropdown(w);
    const opts = w.findAll(".gs-option").map((o) => o.text());
    expect(opts).toContain("glm-5.2");
    expect(opts).toContain("glm-4-flash");
    // 回填的地址随保存固化
    await fillBasics(w, "GLM 助手", "glm-helper");
    await textInputs(w)[2].setValue("sk-glm-key");
    await save(w);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.base_url).toBe(GLM_CODING_URL);
  });

  it("切厂商：自动回填/注册表地址不跟过来（用户手输的代理地址不动）", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 0, models: [], error: null, matched_url: GLM_CODING_URL });
    const w = await mountForm();
    await openDropdown(w);
    await clickOption(w, "glm-5-turbo");
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    expect((baseUrlInput(w).element as HTMLInputElement).value).toBe(GLM_CODING_URL);
    // 切到 deepseek：旧端点清空（走 deepseek 注册表默认），不留跨厂商错配地址
    await openDropdown(w);
    await clickOption(w, "deepseek-chat");
    expect((baseUrlInput(w).element as HTMLInputElement).value).toBe("");
    // 手输代理地址再切厂商：不动（用户明确输入，系统不越权清除）
    await baseUrlInput(w).setValue("https://my-proxy/v1");
    await openDropdown(w);
    await clickOption(w, "glm-5-turbo");
    expect((baseUrlInput(w).element as HTMLInputElement).value).toBe("https://my-proxy/v1");
  });

  it("测试连接失败：红字展示结构化错误（多端点全败聚合文案）", async () => {
    testConnectionMock.mockResolvedValue({
      ok: false, model_count: 0, models: [], matched_url: null,
      error: "全部端点未通过——标准端点：HTTP 401: 认证失败；Coding 端点：HTTP 401: 认证失败",
    });
    const w = await mountForm();
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    const err = w.find(".conn-err");
    expect(err.exists()).toBe(true);
    expect(err.text()).toContain("全部端点未通过");
    expect(err.text()).toContain("标准端点：HTTP 401");
  });

  it("编辑态：可见厂商的存量目录外模型插回所属组；hidden 旧入口（glm-coding）合成兜底组", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 0, models: [], error: null, matched_url: null });
    // 可见厂商 + 目录外模型 → 插回智谱组
    const w = await mountForm(editAgent({ provider: "glm", model: "glm-x-private" }));
    expect(w.find(".gs-value").text()).toBe("glm-x-private");
    await openDropdown(w);
    const owner = w.findAll(".gs-option").find((o) => o.text().includes("glm-x-private"));
    expect(owner).toBeTruthy();

    // hidden 旧入口（glm-coding）→ 单独合成一组显示（不进可见目录组）
    const w2 = await mountForm(editAgent({ provider: "glm-coding", model: "glm-5.2" }));
    expect(w2.find(".gs-value").text()).toBe("glm-5.2");
    expect(testConnectionMock).not.toHaveBeenCalled();
    await w2.find(".conn-btn").trigger("click");
    await flushPromises();
    // 编辑态探测带 agent_id（用存量 Key，密文不回显）
    expect(testConnectionMock).toHaveBeenCalledWith("glm-coding", undefined, undefined, "ag-1");
    await openDropdown(w2);
    const labels = w2.findAll(".gs-group-label").map((l) => l.text());
    expect(labels.filter((l) => l.includes("GLM Coding"))).toHaveLength(1); // 合成兜底组
    expect(labels).toHaveLength(5); // 4 可见组 + 1 合成组
  });
});
