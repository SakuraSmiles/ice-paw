// AgentForm.providers.test.ts — Provider 目录驱动行为锁：
// 下拉来自 bridge.providers.list（后端注册表单一真相源）、
// ollama 空 key 可保存 / custom 缺 base_url 被拦、
// 测试连接成功绿字+模型并入下拉、失败红字、
// 切 provider 清拉取结果且目录选中跟随（编辑态手输模型保留）。
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

// 后端注册表镜像（子集足够驱动表单分支）
const PROVIDERS: ProviderInfo[] = [
  { name: "openai", protocol: "openai", default_url: "https://api.openai.com", label: "OpenAI", note: null, requires_key: true, requires_base_url: false, models: ["gpt-4o", "gpt-4o-mini"] },
  { name: "glm", protocol: "openai", default_url: "https://open.bigmodel.cn/api/paas/v4", label: "智谱 GLM", note: null, requires_key: true, requires_base_url: false, models: ["glm-5-turbo", "glm-4-flash"] },
  { name: "ollama", protocol: "openai", default_url: "http://localhost:11434/v1", label: "Ollama 本地", note: "无需 API Key", requires_key: false, requires_base_url: false, models: [] },
  { name: "custom", protocol: "openai", default_url: "", label: "自定义（OpenAI 兼容）", note: "必填 API URL", requires_key: false, requires_base_url: true, models: [] },
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
const providerInput = (w: VueWrapper) => w.find(".combobox input");
const modelInput = (w: VueWrapper) => w.find(".model-group .combobox input");

/** Combobox 手输精确 label → emit 对应 value（与真实用户操作同路径） */
async function chooseProvider(w: VueWrapper, label: string) {
  await providerInput(w).setValue(label);
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

describe("AgentForm Provider 目录驱动", () => {
  it("Provider 下拉选项来自 bridge.providers.list（含 note 副行）", async () => {
    const w = await mountForm();
    await providerInput(w).trigger("focus");
    const opts = w.findAll(".combobox-option-rich");
    expect(providersListMock).toHaveBeenCalled();
    expect(opts).toHaveLength(PROVIDERS.length);
    const texts = opts.map((o) => o.text());
    expect(texts.join("|")).toContain("Ollama 本地");
    expect(texts.join("|")).toContain("无需 API Key");
  });

  it("ollama（requires_key=false）空 API Key 可直接保存", async () => {
    const w = await mountForm();
    await chooseProvider(w, "Ollama 本地");
    const inputs = textInputs(w);
    await inputs[0].setValue("本地助手");
    await inputs[1].setValue("local-helper");
    await modelInput(w).setValue("qwen3:8b");
    // API Key 留空
    await w.find(".section-actions .btn-primary").trigger("click");
    await flushPromises();
    expect(w.find(".form-error").exists()).toBe(false);
    expect(createMock).toHaveBeenCalledTimes(1);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("ollama");
    expect(input.api_key).toBe("");
    expect(input.model).toBe("qwen3:8b");
  });

  it("custom（requires_base_url）缺 API URL 保存被拦", async () => {
    const w = await mountForm();
    await chooseProvider(w, "自定义（OpenAI 兼容）");
    const inputs = textInputs(w);
    await inputs[0].setValue("自建端点");
    await inputs[1].setValue("my-vllm");
    await modelInput(w).setValue("Qwen3-32B");
    await w.find(".section-actions .btn-primary").trigger("click");
    await flushPromises();
    expect(w.find(".form-error").text()).toContain("自定义 Provider 必须填写 API URL");
    expect(createMock).not.toHaveBeenCalled();
  });

  it("测试连接成功：绿字文案 + 拉取到的模型并入模型下拉", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 2, models: ["gpt-5", "o4-mini"], error: null });
    const w = await mountForm();
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    expect(testConnectionMock).toHaveBeenCalledWith("openai", undefined, undefined, undefined);
    const ok = w.find(".conn-ok");
    expect(ok.exists()).toBe(true);
    expect(ok.text()).toContain("连接成功，发现 2 个模型");
    // 拉取结果与静态目录去重合并
    await modelInput(w).trigger("focus");
    const modelOpts = w.findAll(".model-group .combobox-option").map((o) => o.text());
    expect(modelOpts).toContain("gpt-5");
    expect(modelOpts).toContain("gpt-4o");
  });

  it("测试连接失败：红字展示结构化错误（探测错误不抛异常）", async () => {
    testConnectionMock.mockResolvedValue({ ok: false, model_count: 0, models: [], error: "HTTP 401: invalid api key" });
    const w = await mountForm();
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    const err = w.find(".conn-err");
    expect(err.exists()).toBe(true);
    expect(err.text()).toContain("HTTP 401: invalid api key");
  });

  it("切 provider：编辑态带 agent_id 探测；清拉取结果与测试态；目录选中跟随", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 1, models: ["gpt-5"], error: null });
    const w = await mountForm(editAgent()); // openai + gpt-4o（目录选中）
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    expect(testConnectionMock).toHaveBeenCalledWith("openai", undefined, undefined, "ag-1");
    expect(w.find(".conn-ok").exists()).toBe(true);
    await modelInput(w).trigger("focus");
    expect(w.findAll(".model-group .combobox-option").map((o) => o.text())).toContain("gpt-5");

    // 切到智谱 GLM：测试态清空、gpt-5 不再出现、模型跟随目录首项
    await chooseProvider(w, "智谱 GLM");
    expect(w.find(".conn-ok").exists()).toBe(false);
    expect((modelInput(w).element as HTMLInputElement).value).toBe("glm-5-turbo");
    await modelInput(w).trigger("focus");
    const opts = w.findAll(".model-group .combobox-option").map((o) => o.text());
    expect(opts).not.toContain("gpt-5");
    expect(opts).toContain("glm-4-flash");
  });

  it("切 provider：编辑态手输模型名（非目录来源）不被覆盖", async () => {
    const w = await mountForm(editAgent({ model: "my-finetune" }));
    await chooseProvider(w, "智谱 GLM");
    expect((modelInput(w).element as HTMLInputElement).value).toBe("my-finetune");
  });
});
