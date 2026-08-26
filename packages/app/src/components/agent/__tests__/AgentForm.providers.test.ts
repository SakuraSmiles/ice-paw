// AgentForm.providers.test.ts — 可选可输分组模型选择器行为锁：
// 下拉只含可见厂商（Ollama/custom/旧入口 hidden 不进）、选预设条目 URL 锁定
// 注册表地址、手输目录外名字落 custom（URL 必填可编辑、可免 Key）、测试连接
// 走通地址回填固化（智谱 Coding 备选端点）+ 探测传参规则（值==默认传
// undefined 走多端点回退）、失败红字、编辑态 hidden 旧入口合成兜底组且
// URL 可编辑（存量 Ollama 改端口场景）。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import type { Agent, ProviderInfo } from "../../../types";

// AgentForm 顶层 import 了两个 Tauri 插件（选目录/文件管理器），全局 setup 未覆盖，须先 mock
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));

const providersListMock = vi.fn();
const testConnectionMock = vi.fn();
const createMock = vi.fn();
const updateMock = vi.fn();
const rotateKeyMock = vi.fn();

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
      update: (...a: unknown[]) => updateMock(...a),
      rotateKey: (...a: unknown[]) => rotateKeyMock(...a),
      list: async () => [],
    },
  },
}));

const GLM_STD_URL = "https://open.bigmodel.cn/api/paas/v4";
const GLM_CODING_URL = "https://open.bigmodel.cn/api/coding/paas/v4";

// 后端注册表镜像（子集足够驱动表单分支；ollama/custom/glm-coding 均 hidden——
// Ollama 不进下拉：本地服务地址因人而异，手输模型名 + 填本机 URL 覆盖）
const PROVIDERS: ProviderInfo[] = [
  { name: "openai", protocol: "openai", default_url: "https://api.openai.com", alt_urls: [], label: "OpenAI", note: null, requires_key: true, requires_base_url: false, hidden: false, models: ["gpt-4o", "gpt-4o-mini"] },
  { name: "glm", protocol: "openai", default_url: GLM_STD_URL, alt_urls: [["Coding 端点", GLM_CODING_URL]], label: "智谱", note: null, requires_key: true, requires_base_url: false, hidden: false, models: ["glm-5-turbo", "glm-4-flash"] },
  { name: "deepseek", protocol: "openai", default_url: "https://api.deepseek.com", alt_urls: [], label: "DeepSeek", note: null, requires_key: true, requires_base_url: false, hidden: false, models: ["deepseek-chat"] },
  { name: "ollama", protocol: "openai", default_url: "http://localhost:11434/v1", alt_urls: [], label: "Ollama 本地", note: "已下线", requires_key: false, requires_base_url: false, hidden: true, models: [] },
  { name: "glm-coding", protocol: "openai", default_url: GLM_CODING_URL, alt_urls: [], label: "智谱 GLM Coding", note: "旧入口", requires_key: true, requires_base_url: false, hidden: true, models: ["glm-5.2"] },
  { name: "custom", protocol: "openai", default_url: "", alt_urls: [], label: "自定义（OpenAI 兼容）", note: null, requires_key: false, requires_base_url: true, hidden: true, models: [] },
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

function modelInput(w: VueWrapper) {
  return w.find(".gs-input");
}

function baseUrlInput(w: VueWrapper) {
  return textInputs(w)[3];
}

function baseUrlValue(w: VueWrapper) {
  return (baseUrlInput(w).element as HTMLInputElement).value;
}

function baseUrlLocked(w: VueWrapper) {
  return (baseUrlInput(w).element as HTMLInputElement).readOnly;
}

async function openDropdown(w: VueWrapper) {
  await modelInput(w).trigger("focus");
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
  updateMock.mockResolvedValue({});
  rotateKeyMock.mockResolvedValue(undefined);
});

afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount();
});

describe("AgentForm 可选可输分组模型选择器", () => {
  it("下拉分组只含可见厂商：Ollama/自定义/旧入口 hidden 不进；控件即输入框", async () => {
    const w = await mountForm();
    await openDropdown(w);
    expect(providersListMock).toHaveBeenCalled();
    const labels = w.findAll(".gs-group-label").map((l) => l.text());
    expect(labels).toHaveLength(3); // openai + 智谱 + deepseek
    expect(labels.join("|")).not.toContain("Ollama");
    expect(labels.join("|")).not.toContain("自定义");
    expect(labels.join("|")).not.toContain("GLM Coding");
    // 控件即输入框（无独立搜索框），可输入过滤
    expect(w.find(".gs-input").exists()).toBe(true);
    expect(w.find(".gs-search").exists()).toBe(false);
    // openai 2 + glm 2 + deepseek 1
    expect(w.findAll(".gs-option")).toHaveLength(5);
  });

  it("输入实时过滤目录；模糊命中仍保留「使用自定义模型」逃生口", async () => {
    const w = await mountForm();
    await openDropdown(w);
    expect(w.findAll(".gs-option")).toHaveLength(5); // 展开初期全目录（当前值只回显不过滤）
    await modelInput(w).setValue("glm-5");
    expect(w.findAll(".gs-option").length).toBeLessThan(5); // 键入后过滤
    // 模糊命中（glm-5 ≠ glm-5.2）→ 逃生口保留（真想用这个名字点它即落 custom）
    const custom = w.find(".gs-option-custom");
    expect(custom.exists()).toBe(true);
    expect(custom.text()).toContain("glm-5");
    w.unmount();
  });

  it("选预设条目：provider+model 推导 + URL 锁定注册表地址（readonly），保存带默认地址", async () => {
    const w = await mountForm();
    await openDropdown(w);
    await clickOption(w, "glm-5-turbo");
    await fillBasics(w, "GLM 助手", "glm-helper");
    await textInputs(w)[2].setValue("sk-glm-key");
    // URL 只读 + 值=注册表默认（预设厂商地址由系统管理）
    expect(baseUrlLocked(w)).toBe(true);
    expect(baseUrlValue(w)).toBe(GLM_STD_URL);
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("glm");
    expect(input.model).toBe("glm-5-turbo");
    expect(input.base_url).toBe(GLM_STD_URL);
    // 控件显示选中模型名 + 当前归属厂商图标
    expect((modelInput(w).element as HTMLInputElement).value).toBe("glm-5-turbo");
    expect(w.find(".gs-control svg.provider-icon").exists()).toBe(true);
  });

  it("手输目录外名字落自定义：URL 必填可编辑、免 Key 可保存（Ollama 场景）", async () => {
    const w = await mountForm();
    await openDropdown(w);
    // 输入目录外模型名（如 Ollama 本地模型）→「使用自定义模型」条目
    await modelInput(w).setValue("qwen3:8b");
    const custom = w.find(".gs-option-custom");
    expect(custom.exists()).toBe(true);
    await custom.trigger("click");
    await flushPromises();
    // URL 解锁可编辑 + 必填（空则保存被拦）
    expect(baseUrlLocked(w)).toBe(false);
    expect(baseUrlValue(w)).toBe("");
    await fillBasics(w, "本地助手", "local-helper");
    await save(w);
    expect(w.find(".form-error").text()).toContain("须填写 API URL");
    expect(createMock).not.toHaveBeenCalled();
    // 填本机 Ollama 地址 + 空 Key → 保存成功（provider=custom，OpenAI 兼容协议）
    await baseUrlInput(w).setValue("http://localhost:11434/v1");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("custom");
    expect(input.model).toBe("qwen3:8b");
    expect(input.base_url).toBe("http://localhost:11434/v1");
    expect(input.api_key).toBe("");
    // 控件回显手输模型名（unmatchedLabel）
    expect((modelInput(w).element as HTMLInputElement).value).toBe("qwen3:8b");
  });

  it("预设智谱测试连接：值==默认传 undefined 走多端点回退，走通 Coding 地址回填固化（readonly 不挡系统赋值）", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 1, models: ["glm-5.2"], error: null, matched_url: GLM_CODING_URL });
    const w = await mountForm();
    await openDropdown(w);
    await clickOption(w, "glm-5-turbo");
    await textInputs(w)[2].setValue("sk-coding-key");
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    // URL 显示默认地址时探测传 undefined——后端按 [标准→Coding] 回退自动匹配
    expect(testConnectionMock).toHaveBeenCalledWith("glm", undefined, "sk-coding-key", undefined);
    expect(w.find(".conn-ok").text()).toContain("连接成功，发现 1 个模型");
    // 走通的 Coding 端点回填固化（字段仍 readonly，但系统赋值生效）
    expect(baseUrlLocked(w)).toBe(true);
    expect(baseUrlValue(w)).toBe(GLM_CODING_URL);
    // 拉取结果并入智谱组
    await openDropdown(w);
    const opts = w.findAll(".gs-option").map((o) => o.text());
    expect(opts).toContain("glm-5.2");
    expect(opts).toContain("glm-4-flash");
    // 保存固化走通端点
    await fillBasics(w, "GLM 助手", "glm-helper");
    await save(w);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.base_url).toBe(GLM_CODING_URL);
  });

  it("自定义路径测试连接：显式传用户填的 URL；拉取结果合成「自定义端点」组", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 1, models: ["qwen3:8b", "llama3:70b"], error: null, matched_url: "http://localhost:11434/v1" });
    const w = await mountForm();
    await openDropdown(w);
    await modelInput(w).setValue("placeholder-first");
    await w.find(".gs-option-custom").trigger("click");
    await flushPromises();
    await baseUrlInput(w).setValue("http://localhost:11434/v1");
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    // custom 必传显式地址（后端 requires_base_url 校验）
    expect(testConnectionMock).toHaveBeenCalledWith("custom", "http://localhost:11434/v1", undefined, undefined);
    // 拉到的本机模型合成组，可点选
    await openDropdown(w);
    await clickOption(w, "llama3:70b");
    expect((modelInput(w).element as HTMLInputElement).value).toBe("llama3:70b");
    await fillBasics(w, "本地助手", "local-helper");
    await save(w);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("custom");
    expect(input.model).toBe("llama3:70b");
  });

  it("测试连接失败：红字展示结构化错误（多端点全败聚合文案）", async () => {
    testConnectionMock.mockResolvedValue({
      ok: false, model_count: 0, models: [], matched_url: null,
      error: "全部端点未通过——标准端点：HTTP 401: 认证失败；Coding 端点：HTTP 401: 认证失败",
    });
    const w = await mountForm(); // 默认 openai
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    const err = w.find(".conn-err");
    expect(err.exists()).toBe(true);
    expect(err.text()).toContain("全部端点未通过");
  });

  it("编辑态换厂商必须带新 Key：空 Key 保存被拦（提示厂商名），填后 update+rotateKey 同批生效；同厂商换模型不受限", async () => {
    // 存量 openai agent → 点选智谱模型（provider 随模型切）
    const w = await mountForm(editAgent({ provider: "openai", model: "gpt-4o" }));
    await openDropdown(w);
    await clickOption(w, "glm-5-turbo");
    await save(w);
    // 空 key → 拦 + 提示含新厂商名与 Key 指引
    const errText = w.find(".form-error").text();
    expect(errText).toContain("智谱");
    expect(errText).toContain("API Key");
    expect(updateMock).not.toHaveBeenCalled();
    // 填新 key → 放行：update 与 rotateKey（新 key + 切换后的厂商地址）都被调
    await textInputs(w)[2].setValue("sk-glm-new-key");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    expect(updateMock).toHaveBeenCalledWith(expect.objectContaining({ provider: "glm", model: "glm-5-turbo" }));
    expect(rotateKeyMock).toHaveBeenCalledWith("ag-1", "sk-glm-new-key", GLM_STD_URL);
    w.unmount();

    // 同厂商换模型：key 留空（=不改）照常保存，不被新闸误伤
    updateMock.mockClear();
    rotateKeyMock.mockClear();
    const w2 = await mountForm(editAgent({ provider: "openai", model: "gpt-4o" }));
    await openDropdown(w2);
    await clickOption(w2, "gpt-4o-mini");
    await save(w2);
    expect(w2.find(".form-error").exists()).toBe(false);
    expect(updateMock).toHaveBeenCalledWith(expect.objectContaining({ model: "gpt-4o-mini" }));
    expect(rotateKeyMock).not.toHaveBeenCalled();
  });

  it("编辑态：可见厂商存量目录外模型插回所属组；hidden 旧入口（glm-coding）合成兜底组且 URL 可编辑", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 0, models: [], error: null, matched_url: null });
    // 可见厂商 + 目录外模型 → 插回智谱组（编辑态 URL 锁定 + 存量地址原样）
    const w = await mountForm(editAgent({ provider: "glm", model: "glm-x-private", base_url: GLM_CODING_URL }));
    expect((modelInput(w).element as HTMLInputElement).value).toBe("glm-x-private");
    expect(baseUrlValue(w)).toBe(GLM_CODING_URL); // 存量固化地址原样（≠默认也直接显示）
    await openDropdown(w);
    expect(w.findAll(".gs-option").some((o) => o.text().includes("glm-x-private"))).toBe(true);
    // 探测：存量地址≠默认 → 显式传（只测它，不回退）
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    expect(testConnectionMock).toHaveBeenCalledWith("glm", GLM_CODING_URL, undefined, "ag-1");
    w.unmount();

    // hidden 旧入口（glm-coding）→ 合成兜底组显示；URL 可编辑（hidden 不锁）
    const w2 = await mountForm(editAgent({ provider: "ollama", model: "qwen3:8b", base_url: "http://192.168.1.10:11434/v1" }));
    expect((modelInput(w2).element as HTMLInputElement).value).toBe("qwen3:8b");
    expect(baseUrlLocked(w2)).toBe(false); // 存量 Ollama 改端口/换机器地址仍可编辑
    expect(baseUrlValue(w2)).toBe("http://192.168.1.10:11434/v1");
    await openDropdown(w2);
    const labels = w2.findAll(".gs-group-label").map((l) => l.text());
    expect(labels.some((l) => l.includes("Ollama"))).toBe(true); // 合成兜底组
  });
});
