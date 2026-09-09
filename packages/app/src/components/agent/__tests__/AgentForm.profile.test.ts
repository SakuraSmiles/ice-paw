// AgentForm.profile.test.ts — 模型来源两形态由 referenceMode 决定（ModelProfile
// Phase 3 + 2026-09-09 双入口拍板）：
// - 新建 = 双入口 pills：「引用已有」（默认——实体库有档可复用；暖缓存直取、
//   冷缓存加载完成后回评翻转，沾手 guard 防覆盖）/「手动填写」（GroupedSelect +
//   Key/URL，保存时后端自动物化为「设置-模型」实体 + 引用）
// - 编辑 = 恒「引用已有」：合并 tag 选择器——主模型与降级链同框（首 tag = 主），
//   保存拆装 model_profile_id=首 / fallback_profile_ids=尾；不发 provider/model/
//   base_url（快照列族由后端解析产生）；Key 在 profile 侧维护（无 rotateKey）
// 拖拽排序 = vuedraggable（jsdom 无法模拟原生 DnD——只断言容器渲染，顺序
// 语义由键盘 ←/→ 通道覆盖，两条通道改同一个 chainIds）。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import type { Agent, ModelProfile, ProviderInfo } from "../../../types";

// AgentForm 顶层 import 了两个 Tauri 插件（选目录/文件管理器），全局 setup 未覆盖，须先 mock
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));

// 「＋ 新建模型配置」跳设置-模型页（router push 断言用）
const routerPushMock = vi.hoisted(() => vi.fn());
vi.mock("vue-router", () => ({
  useRouter: () => ({ push: routerPushMock }),
}));

const providersListMock = vi.fn();
const profilesListMock = vi.fn();
const createMock = vi.fn();
const updateMock = vi.fn();
const rotateKeyMock = vi.fn();
const testModelChainMock = vi.fn();

// useProviders / useModelProfiles / AgentForm 各自从不同相对路径 import 同一 bridge 模块——一份 mock 全覆盖
vi.mock("../../../api/bridge", () => ({
  bridge: {
    providers: {
      list: (...a: unknown[]) => providersListMock(...a),
      testModelChain: (...a: unknown[]) => testModelChainMock(...a),
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

// 后端注册表镜像（子集：下拉候选的厂商展示名解析用）
const PROVIDERS: ProviderInfo[] = [
  { name: "openai", protocol: "openai", default_url: "https://api.openai.com", alt_urls: [], label: "OpenAI", note: null, requires_key: true, requires_base_url: false, key_url: null, openai_url: "https://api.openai.com", hidden: false, models: ["gpt-4o"] },
  { name: "glm", protocol: "openai", default_url: "https://open.bigmodel.cn/api/paas/v4", alt_urls: [], label: "智谱", note: null, requires_key: true, requires_base_url: false, key_url: null, openai_url: "https://open.bigmodel.cn/api/paas/v4", hidden: false, models: ["glm-5.3"] },
  { name: "custom", protocol: "openai", default_url: "", alt_urls: [], label: "自定义（OpenAI 兼容）", note: null, requires_key: false, requires_base_url: true, key_url: null, openai_url: null, hidden: true, models: [] },
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
    has_api_key: true, last_health: "rate_limited", last_health_detail: null, last_health_at: "2026-09-08 00:00:00",
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

function tags(w: VueWrapper) {
  return w.findAll(".chain-tag");
}

/** 点开候选下拉（box 即触发面） */
async function openMenu(w: VueWrapper) {
  await w.find(".chain-box").trigger("click");
  await flushPromises();
}

/** 点候选条目（按别名文本找，等价真实用户点选） */
async function pickCandidate(w: VueWrapper, alias: string) {
  const el = w.findAll(".chain-option").find((o) => o.text().includes(alias));
  expect(el, `候选应含「${alias}」`).toBeTruthy();
  await el!.trigger("click");
  await flushPromises();
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

describe("AgentForm 编辑态配置链（合并 tag 选择器）", () => {
  it("编辑回显 tag 链：首 tag 带（主）badge + 语义健康点；手动区整块不渲染", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1", fallback_profile_ids: ["mp-2"] }));
    // 手动区整块不渲染（分组选择器 + Key/URL 行 + 手动测试连接）；
    // 引用态测试按钮在 .chain-test-row（链路仿真，新位置）
    expect(w.find(".gs-input").exists()).toBe(false);
    expect(w.find("input.input[type=password]").exists()).toBe(false);
    expect(w.find(".label-row .conn-btn").exists()).toBe(false);
    expect(w.find(".chain-test-row .conn-btn").exists()).toBe(true);
    // 链语义 hint 塞进 label 问号 tip（2026-09-09 ③）
    expect(w.find(".label-row .tip-icon").attributes("data-tip")).toContain("主模型");
    // 两枚 tag：首 = 智谱主力（主 badge + success 点），次 = 本地 Qwen（warning 点）
    const ts = tags(w);
    expect(ts).toHaveLength(2);
    expect(ts[0].text()).toContain("智谱主力");
    expect(ts[0].text()).toContain("主");
    expect(ts[0].classes()).toContain("chain-tag--main");
    expect(ts[0].find(".tag-dot--success").exists()).toBe(true);
    expect(ts[1].text()).toContain("本地 Qwen");
    expect(ts[1].find(".tag-dot--warning").exists()).toBe(true);
    // 拖拽容器渲染到位（vuedraggable 根即 tag 容器；jsdom 无法模拟原生 DnD，
    // 顺序语义由下方键盘 ←/→ 用例覆盖）
    expect(w.find(".chain-tags").exists()).toBe(true);
  });

  it("保存拆装链：model_profile_id=首、fallback_profile_ids=尾；不发快照三件、无 rotateKey", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1", fallback_profile_ids: ["mp-2"] }));
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    expect(updateMock).toHaveBeenCalledTimes(1);
    const input = updateMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBe("mp-1");
    expect(input.fallback_profile_ids).toEqual(["mp-2"]);
    expect(input.name).toBe("助手");
    // 快照列族由后端解析产生——同批发会被冲突校验拒，本表单不持有
    expect("provider" in input).toBe(false);
    expect("model" in input).toBe(false);
    expect("base_url" in input).toBe(false);
    // Key 在 profile 侧维护——不走 agent 槽位轮换
    expect(rotateKeyMock).not.toHaveBeenCalled();
  });

  it("下拉候选排除已在链项；点选追加到链尾并随批提交", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1" }));
    await openMenu(w);
    // 候选只剩未入链的 mp-2（选一少一，天然去重）
    const options = w.findAll(".chain-option");
    expect(options).toHaveLength(2); // mp-2 候选 + 「＋ 新建模型配置」入口
    expect(options.some((o) => o.text().includes("智谱主力"))).toBe(false);
    await pickCandidate(w, "本地 Qwen");
    expect(tags(w)).toHaveLength(2);
    expect(tags(w)[1].text()).toContain("本地 Qwen"); // 追加到链尾
    await save(w);
    const input = updateMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.fallback_profile_ids).toEqual(["mp-2"]);
  });

  it("移除主档 → 第二 tag 升任主模型（badge 转移），保存引用随链头换档", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1", fallback_profile_ids: ["mp-2"] }));
    await tags(w)[0].find(".tag-x").trigger("click");
    await flushPromises();
    const ts = tags(w);
    expect(ts).toHaveLength(1);
    expect(ts[0].text()).toContain("本地 Qwen");
    expect(ts[0].classes()).toContain("chain-tag--main");
    await save(w);
    const input = updateMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBe("mp-2");
    expect(input.fallback_profile_ids).toEqual([]);
  });

  it("清空链保存被拦：提示至少选择一个模型配置（≥1 校验）", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1" }));
    await tags(w)[0].find(".tag-x").trigger("click");
    await flushPromises();
    expect(w.find(".chain-placeholder").exists()).toBe(true);
    await save(w);
    expect(w.find(".form-error").text()).toContain("至少选择一个模型配置");
    expect(updateMock).not.toHaveBeenCalled();
  });

  it("键盘 ←/→ 换位（无障碍通道，与拖拽同改 chainIds）：保存序随换位变", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1", fallback_profile_ids: ["mp-2"] }));
    // 首 tag 按 →：与右邻交换，主 badge 转移到 mp-2
    await tags(w)[0].trigger("keydown", { key: "ArrowRight" });
    await flushPromises();
    const ts = tags(w);
    expect(ts[0].text()).toContain("本地 Qwen");
    expect(ts[0].classes()).toContain("chain-tag--main");
    expect(ts[1].text()).toContain("智谱主力");
    // ← 换回来（焦点跟人走：换位后焦点在换过去的 tag 上，即 index 1）
    await tags(w)[1].trigger("keydown", { key: "ArrowLeft" });
    await flushPromises();
    expect(tags(w)[0].text()).toContain("智谱主力");
    await save(w);
    const input = updateMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBe("mp-1");
    expect(input.fallback_profile_ids).toEqual(["mp-2"]);
  });

  it("悬空引用（profile 已删）诚实标注不冒充；移除后重选恢复可用", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-gone" }));
    expect(tags(w)[0].text()).toContain("配置已删除");
    expect(tags(w)[0].find(".tag-dot--neutral").exists()).toBe(true);
    // 移除悬空档 → 重选健康档 → 保存引用修复
    await tags(w)[0].find(".tag-x").trigger("click");
    await flushPromises();
    await openMenu(w);
    await pickCandidate(w, "智谱主力");
    await save(w);
    const input = updateMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBe("mp-1");
  });

  it("「＋ 新建模型配置」跳设置-模型页（编辑想换全新配置由该入口兜住）", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1" }));
    await openMenu(w);
    const newBtn = w.find(".chain-option--new");
    expect(newBtn.exists()).toBe(true);
    expect(newBtn.text()).toContain("新建模型配置");
    await newBtn.trigger("click");
    await flushPromises();
    expect(routerPushMock).toHaveBeenCalledWith({ name: "SettingsModels" });
    expect(w.find(".chain-menu").exists()).toBe(false); // 跳转即收起
  });

  it("legacy 行（无引用）编辑：空链待选，placeholder 可见；选中后保存即转正", async () => {
    const w = await mountForm(editAgent({ model_profile_id: undefined, fallback_profile_ids: undefined }));
    expect(tags(w)).toHaveLength(0);
    expect(w.find(".chain-placeholder").text()).toContain("选择模型配置");
    await openMenu(w);
    await pickCandidate(w, "智谱主力");
    await save(w);
    const input = updateMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBe("mp-1");
    expect(input.fallback_profile_ids).toEqual([]);
  });
});

describe("AgentForm 新建态双入口（引用默认 / 手动档）", () => {
  it("新建手动档 = GroupedSelect + Key/URL，无链选择器；create 载荷不含引用字段（后端自动物化）", async () => {
    const w = await mountForm(null);
    // 双入口默认「引用已有」（profiles 已加载、回评翻转已发生）——先切手动档走手写流程
    await w.findAll(".mode-pill")[1].trigger("click");
    await flushPromises();
    expect(w.find(".gs-input").exists()).toBe(true);
    expect(w.find("input.input[type=password]").exists()).toBe(true);
    expect(w.find(".chain-box").exists()).toBe(false);
    // 可见性提示（转换要可见）：保存即转实体
    expect(w.find(".label-note").text()).toContain("设置-模型");
    // 走 custom 通道填齐（免 Key 本机端点）保存
    await w.find(".gs-input").trigger("focus");
    await w.find(".gs-input").setValue("qwen3:8b");
    await w.find(".gs-option-custom").trigger("click");
    await flushPromises();
    const inputs = w.findAll("input.input").filter((i) => !i.element.classList.contains("workspace-input"));
    await inputs[0].setValue("本地助手");
    await inputs[1].setValue("local-helper");
    await inputs[3].setValue("http://localhost:11434/v1");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.provider).toBe("custom");
    expect(input.model).toBe("qwen3:8b");
    expect("model_profile_id" in input).toBe(false); // 物化在后端事务内发生
    expect("fallback_profile_ids" in input).toBe(false);
  });

  it("新建默认「引用已有」：profiles 加载完成自动翻档，选档保存发引用载荷（空串占位）", async () => {
    const w = await mountForm(null);
    // mountForm 内 flushPromises 已覆盖回评翻转：链选择器渲染、手动区不渲染
    expect(w.find(".chain-box").exists()).toBe(true);
    expect(w.find(".gs-input").exists()).toBe(false);
    expect(w.find("input.input[type=password]").exists()).toBe(false);
    // pills：引用已有 active（[0]=引用已有 / [1]=手动填写）
    const pills = w.findAll(".mode-pill");
    expect(pills).toHaveLength(2);
    expect(pills[0].classes()).toContain("active");
    // 填出生证 + 选档 + 保存
    const inputs = w.findAll("input.input").filter((i) => !i.element.classList.contains("workspace-input"));
    await inputs[0].setValue("引用助手");
    await inputs[1].setValue("ref-helper");
    await openMenu(w);
    await pickCandidate(w, "智谱主力");
    await save(w);
    expect(w.find(".form-error").exists()).toBe(false);
    expect(createMock).toHaveBeenCalledTimes(1);
    const input = createMock.mock.calls[0][0] as Record<string, unknown>;
    expect(input.model_profile_id).toBe("mp-1");
    expect(input.fallback_profile_ids).toEqual([]);
    // 空串占位：后端引用模式跳过必填（快照列由 profile 解析产生）
    expect(input.provider).toBe("");
    expect(input.model).toBe("");
    expect(input.api_key).toBe("");
  });

  it("沾手不回翻：加载完成前已填模型 → 保持手动档（guard 防覆盖用户输入）", async () => {
    // profiles 加载挂起（受控 Promise）——mount 后表单停在初值 manual
    let resolveProfiles!: (v: ModelProfile[]) => void;
    profilesListMock.mockImplementation(
      () => new Promise<ModelProfile[]>((r) => { resolveProfiles = r; }),
    );
    const w = await mountForm(null);
    expect(w.find(".gs-input").exists()).toBe(true);
    // 用户先手填了模型（沾手：guard 判据字段非空）
    await w.find(".gs-input").trigger("focus");
    await w.find(".gs-input").setValue("qwen3:8b");
    await w.find(".gs-option-custom").trigger("click");
    await flushPromises();
    // profiles 此时才加载完成 → 回评 guard 拦住，不翻引用档
    resolveProfiles(PROFILES);
    await flushPromises();
    expect(w.find(".gs-input").exists()).toBe(true);
    expect(w.find(".chain-box").exists()).toBe(false);
  });

  it("链选择器开合：空白区点击切换、tag 点击止冒泡不开合、外点关闭（#1/#2）", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1" }));
    // 空白区（tag 容器空位）点击冒泡到 box = 展开
    await w.find(".chain-tags").trigger("click");
    expect(w.find(".chain-menu").exists()).toBe(true);
    // tag 自身点击止冒泡——不切换开合（它是拖拽/键盘目标，菜单保持开）
    await tags(w)[0].trigger("click");
    await flushPromises();
    expect(w.find(".chain-menu").exists()).toBe(true);
    // 外点（wrap 之外）关闭
    document.body.click();
    await flushPromises();
    expect(w.find(".chain-menu").exists()).toBe(false);
    // box 空白点击 = 再展开；开态再点 = 收起（toggle 双向）
    await w.find(".chain-box").trigger("click");
    expect(w.find(".chain-menu").exists()).toBe(true);
    await w.find(".chain-box").trigger("click");
    expect(w.find(".chain-menu").exists()).toBe(false);
  });

  it("链路测试（④）：按链序发 testModelChain；降级命中文案注明，链变旧结论失效", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1", fallback_profile_ids: ["mp-2"] }));
    testModelChainMock.mockResolvedValue({
      ok: true, profile_id: "mp-2", alias: "本地 Qwen", model: "qwen3:8b", error: null,
    });
    await w.find(".chain-test-row .conn-btn").trigger("click");
    await flushPromises();
    // 载荷 = 链序快照（主模型在前）
    expect(testModelChainMock).toHaveBeenCalledWith(["mp-1", "mp-2"]);
    const txt = w.find(".chain-test-row .conn-ok").text();
    expect(txt).toContain("主模型不可用"); // 非首档命中 → 降级注明（主模型该看一眼）
    expect(txt).toContain("本地 Qwen");
    // 链一变（移除一档）旧结论即失效
    await tags(w)[1].find(".tag-x").trigger("click");
    await flushPromises();
    expect(w.find(".chain-test-row .conn-ok").exists()).toBe(false);
  });

  it("链路测试（④）：全链失败红字透出最后错误", async () => {
    const w = await mountForm(editAgent({ model_profile_id: "mp-1" }));
    testModelChainMock.mockResolvedValue({
      ok: false, profile_id: null, alias: null, model: null,
      error: "链上 1 个档位全部不可用——最后错误：「智谱主力」：HTTP 401: unauthorized",
    });
    await w.find(".chain-test-row .conn-btn").trigger("click");
    await flushPromises();
    expect(w.find(".chain-test-row .conn-err").text()).toContain("401");
  });
});
