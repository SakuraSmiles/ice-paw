// AgentForm.profile.test.ts — 模型来源三态（ModelProfile Phase 2 批 1）：
// 编辑按行引用列初始化引用模式；引用区 = Combobox 选实体 + 摘要（厂商/
// 模型/Key 态/健康点）+ 悬空引用降级提示；模型选择器与 Key/URL 行只在
// 手动形态渲染（换厂商 Key 闸引用模式天然消失）；保存分叉——引用只发
// model_profile_id（不发 provider/model/base_url，后端冲突校验拦同批）、
// 手动显式发 null 解除引用。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import type { Agent, ModelProfile, ProviderInfo } from "../../../types";

// AgentForm 顶层 import 了两个 Tauri 插件（选目录/文件管理器），全局 setup 未覆盖，须先 mock
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));

const providersListMock = vi.fn();
const profilesListMock = vi.fn();
const createMock = vi.fn();
const updateMock = vi.fn();
const rotateKeyMock = vi.fn();

// useProviders / useModelProfiles / AgentForm 各自从不同相对路径 import 同一 bridge 模块——一份 mock 全覆盖
vi.mock("../../../api/bridge", () => ({
  bridge: {
    providers: {
      list: (...a: unknown[]) => providersListMock(...a),
    },
    preferences: { get: async () => ({}) },
    agents: {
      create: (...a: unknown[]) => createMock(...a),
      update: (...a: unknown[]) => updateMock(...a),
      rotateKey: (...a: unknown[]) => rotateKeyMock(...a),
      list: async () => [],
    },
    modelProfiles: { list: (...a: unknown[]) => profilesListMock(...a) },
  },
}));

// 后端注册表镜像（子集：摘要行的厂商展示名解析用）
const PROVIDERS: ProviderInfo[] = [
  { name: "openai", protocol: "openai", default_url: "https://api.openai.com", alt_urls: [], label: "OpenAI", note: null, requires_key: true, requires_base_url: false, key_url: null, openai_url: "https://api.openai.com", hidden: false, models: ["gpt-4o"] },
  { name: "glm", protocol: "openai", default_url: "https://open.bigmodel.cn/api/paas/v4", alt_urls: [], label: "智谱", note: null, requires_key: true, requires_base_url: false, key_url: null, openai_url: "https://open.bigmodel.cn/api/paas/v4", hidden: false, models: ["glm-5.3"] },
];

const PROFILES: ModelProfile[] = [
  {
    id: "mp-1", alias: "智谱主力", provider: "glm", model: "glm-5.3",
    base_url: null, sort_order: 0,
    created_at: "2026-09-08 00:00:00", updated_at: "2026-09-08 00:00:00",
    has_api_key: true, last_health: "ok", last_health_detail: null, last_health_at: "2026-09-08 00:00:00",
  },
  {
    id: "mp-2", alias: "本地 Qwen", provider: "custom", model: "qwen3:8b",
    base_url: "http://localhost:11434/v1", sort_order: 1,
    created_at: "2026-09-08 00:00:00", updated_at: "2026-09-08 00:00:00",
    has_api_key: true, last_health: null, last_health_detail: null, last_health_at: null,
  },
];

function editAgent(overrides?: Partial<Agent>): Agent {
  return {
    id: "ag-1",
    name: "助手",
    provider: "glm",
    model: "glm-5.3",
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

/** useProviders/useModelProfiles 是模块级单例缓存——resetModules + 动态 import 保证每个用例拿到干净状态 */
async function mountForm(agent: Agent | null = null) {
  const { default: AgentForm } = await import("../AgentForm.vue");
  const w = mount(AgentForm, { props: { agent }, attachTo: document.body });
  wrappers.push(w);
  await flushPromises();
  return w;
}

/** 模式切换钮：[手动配置, 引用模型配置] */
function modeOpts(w: VueWrapper) {
  return w.findAll(".mode-opt");
}

async function save(w: VueWrapper) {
  await w.find(".section-actions .btn-primary").trigger("click");
  await flushPromises();
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.resetModules();
  providersListMock.mockResolvedValue(PROVIDERS);
  profilesListMock.mockResolvedValue(PROFILES);
  createMock.mockResolvedValue({});
  updateMock.mockResolvedValue({});
  rotateKeyMock.mockResolvedValue(undefined);
});

afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount();
});

describe("AgentForm 模型来源三态（引用/手动）", () => {
  it("编辑态带 model_profile_id 初始化为引用模式：手动区不渲染，摘要含厂商/模型/健康点，选择器回显实体别名", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1" }));
    // 手动区整块不渲染（分组选择器 + Key/URL 行）
    expect(w.find(".gs-input").exists()).toBe(false);
    expect(w.find("input.input[type=password]").exists()).toBe(false);
    // Combobox 回显实体 label（items 异步到位后重查 label）
    const cb = w.find(".combobox-input");
    expect((cb.element as HTMLInputElement).value).toContain("智谱主力");
    // 摘要：厂商展示名 + 模型 + 健康点（语义圆点文字）
    const summary = w.find(".profile-summary");
    expect(summary.exists()).toBe(true);
    expect(summary.text()).toContain("智谱");
    expect(summary.text()).toContain("glm-5.3");
    expect(summary.text()).toContain("正常");
    // 切换钮高亮引用档
    const opts = modeOpts(w);
    expect(opts[1].classes()).toContain("active");
  });

  it("引用模式保存只发出生证 + model_profile_id：不发 provider/model/base_url（后端冲突校验拦同批）、无 rotateKey", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1" }));
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    expect(updateMock).toHaveBeenCalledTimes(1);
    const input = updateMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBe("mp-1");
    expect(input.name).toBe("助手");
    expect("provider" in input).toBe(false);
    expect("model" in input).toBe(false);
    expect("base_url" in input).toBe(false);
    // Key 在 profile 侧维护——引用模式不走 agent 槽位轮换
    expect(rotateKeyMock).not.toHaveBeenCalled();
  });

  it("切回手动保存显式解除引用（model_profile_id=null）：快照列升为权威，provider/model 预填随批提交", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1" }));
    await modeOpts(w)[0].trigger("click");
    await flushPromises();
    // 手动区回来了
    expect(w.find(".gs-input").exists()).toBe(true);
    expect(w.find("input.input[type=password]").exists()).toBe(true);
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = updateMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBeNull();
    // 快照列（后端保持新鲜）随批提交——切回手动一并完成
    expect(input.provider).toBe("glm");
    expect(input.model).toBe("glm-5.3");
  });

  it("换厂商 Key 闸在引用模式天然消失：无 Key/模型手输区，快照厂商与 profile 厂商不同也直接保存", async () => {
    // 行快照是 openai、引用的 profile 是 glm——引用模式下模型身份整体来自
    // profile，不存在「拿旧厂商 key 打新端点」的分裂面，闸不适用
    const w = await mountForm(
      editAgent({ provider: "openai", model: "gpt-4o", model_profile_id: "mp-1" }),
    );
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    expect(updateMock).toHaveBeenCalledWith(expect.objectContaining({ model_profile_id: "mp-1" }));
    expect(rotateKeyMock).not.toHaveBeenCalled();
  });

  it("悬空引用（profile 已删）显示降级提示，不静默清引用、不阻断保存", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-gone" }));
    const hint = w.find(".profile-dangling");
    expect(hint.exists()).toBe(true);
    expect(hint.text()).toContain("已不存在");
    // 降级提示不阻断保存（后端悬空降级行内快照继续可用；删除守卫正常会拦此态）
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    expect(updateMock).toHaveBeenCalledWith(expect.objectContaining({ model_profile_id: "mp-gone" }));
  });

  it("新建默认手动；切引用后下拉选实体创建：入参带 model_profile_id（豁免模型/Key 必填）", async () => {
    const w = await mountForm(null);
    // 新建默认手动形态
    expect(w.find(".gs-input").exists()).toBe(true);
    expect(w.find(".combobox-input").exists()).toBe(false);
    // 切引用 → Combobox 展开点选「本地 Qwen」
    await modeOpts(w)[1].trigger("click");
    await flushPromises();
    const cb = w.find(".combobox-input");
    await cb.trigger("focus");
    await flushPromises();
    const opt = w.findAll(".combobox-option").find((o) => o.text().includes("本地 Qwen"));
    expect(opt, "下拉应含「本地 Qwen」").toBeTruthy();
    await opt!.trigger("click");
    await flushPromises();
    // 出生证两字段填齐（引用模式豁免模型/Key 必填——不填 Key 也能保存）
    const inputs = w.findAll("input.input").filter((i) => !i.element.classList.contains("workspace-input"));
    await inputs[0].setValue("本地助手");
    await inputs[1].setValue("local-helper");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBe("mp-2");
    expect(input.api_key).toBe("");
  });
});
