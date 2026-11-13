import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { open } from "@tauri-apps/plugin-dialog";
import type { Agent, ModelProfile, ProviderInfo } from "../../../types";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));
vi.mock("vue-router", () => ({ useRouter: () => ({ push: vi.fn() }) }));

const mocks = vi.hoisted(() => ({
  openPath: vi.fn(),
  create: vi.fn(),
  update: vi.fn(),
  yamlFields: vi.fn(),
  setSystemPrompt: vi.fn(),
}));

const PROVIDERS: ProviderInfo[] = [{
  name: "openai", protocol: "openai", default_url: "https://api.openai.com/v1",
  alt_urls: [], label: "OpenAI", note: null, requires_key: true, requires_base_url: false,
  key_url: null, openai_url: "https://api.openai.com/v1", hidden: false, models: ["gpt-4o"],
}];
const PROFILES: ModelProfile[] = [{
  id: "mp-1", alias: "主模型", provider: "openai", model: "gpt-4o", base_url: null,
  sort_order: 0, created_at: "2026-11-13 00:00:00", updated_at: "2026-11-13 00:00:00",
  has_api_key: true,
}];

vi.mock("../../../api/bridge", () => ({
  bridge: {
    providers: { list: async () => PROVIDERS },
    modelProfiles: { list: async () => PROFILES },
    preferences: { get: async () => ({}) },
    agents: { ...mocks, list: async () => [] },
    appInfo: { openPath: mocks.openPath },
    mcp: { listBuiltinTools: async () => [], list: async () => [] },
  },
}));

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: "ag-1", name: "助手", provider: "openai", model: "gpt-4o", system_prompt: "",
    base_url: null, temperature: 0.7, max_tokens: 16384, extra_params: {}, sort_order: 0,
    cache_prompt: true, workspace_path: "D:/project-a", config_dir: "C:\\IcePaw\\agents\\ag-1\\",
    config_from_file: true, model_profile_id: "mp-1", has_api_key: true,
    created_at: "2026-11-13 00:00:00", updated_at: "2026-11-13 00:00:00", ...overrides,
  };
}

const wrappers: VueWrapper[] = [];
async function mountForm(value: Agent | null) {
  const { default: AgentForm } = await import("../AgentForm.vue");
  const wrapper = mount(AgentForm, {
    props: { agent: value }, attachTo: document.body, global: { stubs: { teleport: true } },
  });
  wrappers.push(wrapper);
  await flushPromises();
  return wrapper;
}
function presetButton(wrapper: VueWrapper) {
  return wrapper.findAll(".ws-preset-btn").find((b) => b.text() === "风格预设")!;
}

beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  mocks.openPath.mockResolvedValue(undefined);
  mocks.create.mockResolvedValue(agent());
  mocks.update.mockResolvedValue(agent());
  mocks.yamlFields.mockResolvedValue({ system_prompt: null });
  mocks.setSystemPrompt.mockResolvedValue({});
  vi.mocked(open).mockResolvedValue("D:/project-b");
});
afterEach(() => { while (wrappers.length) wrappers.pop()?.unmount(); });

describe("AgentForm 配置目录与执行工作区分离", () => {
  it("修改工作区草稿与保存均不改变配置文件入口，更新载荷不含 config_dir", async () => {
    const w = await mountForm(agent());
    await w.find(".config-open-btn").trigger("click");
    await flushPromises();
    expect(mocks.openPath).toHaveBeenLastCalledWith("C:/IcePaw/agents/ag-1/agent.yaml");

    await w.find(".ws-btn[title='选择目录']").trigger("click");
    await flushPromises();
    expect((w.find(".workspace-input").element as HTMLInputElement).value).toBe("D:/project-b");
    await w.find(".config-open-btn").trigger("click");
    await flushPromises();
    expect(mocks.openPath).toHaveBeenLastCalledWith("C:/IcePaw/agents/ag-1/agent.yaml");

    await w.find(".section-actions .btn-primary").trigger("click");
    await flushPromises();
    expect(mocks.update).toHaveBeenCalledWith(expect.objectContaining({ workspace_path: "D:/project-b" }));
    expect(mocks.update.mock.calls[0][0]).not.toHaveProperty("config_dir");
    expect(w.text()).not.toContain("在此目录下创建");
  });

  it("未指定工作区但有配置目录时仍能打开 YAML 和显式插入风格预设", async () => {
    const w = await mountForm(agent({ workspace_path: null }));
    expect(w.find(".ws-btn-open").exists()).toBe(false);
    expect(w.find(".config-open-btn").exists()).toBe(true);
    await presetButton(w).trigger("click");
    await flushPromises();
    expect(mocks.yamlFields).toHaveBeenCalledWith("ag-1");
    await w.find(".sp-btn.primary").trigger("click");
    await flushPromises();
    expect(mocks.setSystemPrompt).toHaveBeenCalledWith("ag-1", expect.stringContaining("助手"));
    expect(w.text()).toContain("已写入 agent.yaml");
  });

  it("配置目录缺失时不从执行工作区猜测 YAML 入口", async () => {
    const w = await mountForm(agent({ config_dir: null }));
    expect(w.find(".config-open-btn").exists()).toBe(false);
    expect(presetButton(w)).toBeUndefined();
    expect(w.find(".ws-btn-open").exists()).toBe(true);
  });

  it("新建时无工作区仍可选择出生预设，提交只带 system_prompt 不带 config_dir", async () => {
    const w = await mountForm(null);
    const identity = w.findAll(".identity-fields input.input");
    await identity[0].setValue("新助手");
    await identity[1].setValue("new-agent");
    await w.find(".chain-box").trigger("click");
    await w.find(".chain-option").trigger("click");
    await presetButton(w).trigger("click");
    await flushPromises();
    await w.find(".sp-btn.primary").trigger("click");
    await w.find(".section-actions .btn-primary").trigger("click");
    await flushPromises();

    expect(mocks.create).toHaveBeenCalledOnce();
    expect(mocks.create.mock.calls[0][0]).toMatchObject({
      id: "new-agent", name: "新助手", model_profile_id: "mp-1",
      system_prompt: expect.stringContaining("新助手"),
    });
    expect(mocks.create.mock.calls[0][0]).not.toHaveProperty("config_dir");
    expect(mocks.create.mock.calls[0][0].workspace_path).toBeUndefined();
  });

  it("配置文件打开失败给出可见原因与重试指引", async () => {
    mocks.openPath.mockRejectedValueOnce(new Error("文件不存在"));
    const w = await mountForm(agent());
    await w.find(".config-open-btn").trigger("click");
    await flushPromises();
    expect(w.find("[role='alert']").text()).toContain("文件不存在");
    expect(w.find("[role='alert']").text()).toContain("重试");
    expect(w.find(".config-open-btn").attributes("disabled")).toBeUndefined();
  });
});
