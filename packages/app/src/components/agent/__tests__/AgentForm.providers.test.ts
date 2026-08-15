// AgentForm.providers.test.ts — 分组模型下拉（Provider+模型合并）行为锁：
// 目录分组来自 bridge.providers.list、点条目推导 provider+model、组头=厂商入口
// （其后手输归该厂商）、手输目录外=custom、手输目录内自动归属、
// 测试连接成功绿字+拉取模型并入所属组、失败红字、编辑态存量模型兜底条目。
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
const modelInput = (w: VueWrapper) => w.find(".model-group .combobox input");

async function openDropdown(w: VueWrapper) {
  await modelInput(w).trigger("focus");
}

/** 点分组下拉里的条目/组头（按文本找，等价真实用户点选） */
async function clickOption(w: VueWrapper, selector: string, text: string) {
  const el = w.findAll(selector).find((o) => o.text().includes(text));
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

describe("AgentForm 分组模型下拉", () => {
  it("下拉分组来自 bridge.providers.list：组头=厂商，组内=静态目录", async () => {
    const w = await mountForm();
    await openDropdown(w);
    expect(providersListMock).toHaveBeenCalled();
    const heads = w.findAll(".combobox-group-head");
    expect(heads).toHaveLength(PROVIDERS.length);
    const headText = heads.map((h) => h.text()).join("|");
    expect(headText).toContain("Ollama 本地");
    expect(headText).toContain("无需 API Key");
    expect(w.findAll(".combobox-group-item")).toHaveLength(4); // openai 2 + glm 2
  });

  it("点模型条目：provider+model 同时推导，保存走正确归属", async () => {
    const w = await mountForm();
    await openDropdown(w);
    await clickOption(w, ".combobox-group-item", "glm-5-turbo");
    await fillBasics(w, "GLM 助手", "glm-helper");
    await textInputs(w)[2].setValue("sk-glm-key");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("glm");
    expect(input.model).toBe("glm-5-turbo");
  });

  it("点组头（Ollama）→ 手输模型归该厂商，空 API Key 可保存", async () => {
    const w = await mountForm();
    await openDropdown(w);
    await clickOption(w, ".combobox-group-head", "Ollama 本地");
    await modelInput(w).setValue("qwen3:8b"); // 组头上下文内手输 → 归 ollama 非 custom
    await fillBasics(w, "本地助手", "local-helper");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("ollama");
    expect(input.api_key).toBe("");
    expect(input.model).toBe("qwen3:8b");
  });

  it("手输目录外模型名 = 自定义，缺 API URL 保存被拦；补 URL 后可保存", async () => {
    const w = await mountForm();
    await openDropdown(w);
    await modelInput(w).setValue("Qwen3-32B"); // 不在任何目录 → custom
    await fillBasics(w, "自建端点", "my-vllm");
    await save(w);
    expect(w.find(".form-error").text()).toContain("自定义 Provider 必须填写 API URL");
    expect(createMock).not.toHaveBeenCalled();
    expect(w.find(".field-hint").text()).toContain("自定义"); // 归属小字实时反映
  });

  it("手输目录内模型名（精确）自动归属对应厂商", async () => {
    const w = await mountForm();
    await openDropdown(w);
    await modelInput(w).setValue("glm-4-flash");
    await fillBasics(w, "GLM 助手", "glm-2");
    await textInputs(w)[2].setValue("sk-glm-key");
    await save(w);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("glm");
    expect(input.model).toBe("glm-4-flash");
  });

  it("测试连接成功：绿字文案 + 拉取模型并入当前厂商组", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 1, models: ["gpt-5"], error: null });
    const w = await mountForm(); // 默认 openai
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    expect(testConnectionMock).toHaveBeenCalledWith("openai", undefined, undefined, undefined);
    const ok = w.find(".conn-ok");
    expect(ok.exists()).toBe(true);
    expect(ok.text()).toContain("连接成功，发现 1 个模型");
    await openDropdown(w);
    const opts = w.findAll(".combobox-group-item").map((o) => o.text());
    expect(opts).toContain("gpt-5");
    expect(opts).toContain("gpt-4o"); // 静态目录仍在
  });

  it("测试连接失败：红字展示结构化错误（探测错误不抛异常）", async () => {
    testConnectionMock.mockResolvedValue({ ok: false, model_count: 0, models: [], error: "HTTP 401: 认证失败" });
    const w = await mountForm();
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    const err = w.find(".conn-err");
    expect(err.exists()).toBe(true);
    expect(err.text()).toContain("HTTP 401");
  });

  it("编辑态：存量模型显示为所属组兜底条目；探测带 agent_id 用存量 Key", async () => {
    testConnectionMock.mockResolvedValue({ ok: true, model_count: 0, models: [], error: null });
    const w = await mountForm(editAgent({ provider: "glm", model: "glm-x-private" })); // 目录外
    expect((modelInput(w).element as HTMLInputElement).value).toBe("glm-x-private");
    await w.find(".conn-btn").trigger("click");
    await flushPromises();
    expect(testConnectionMock).toHaveBeenCalledWith("glm", undefined, undefined, "ag-1");
    await openDropdown(w);
    const owner = w.findAll(".combobox-group-item").find((o) => o.text().includes("glm-x-private"));
    expect(owner).toBeTruthy(); // 兜底条目插在智谱组，不是自定义组
  });
});
