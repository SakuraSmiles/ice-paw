// ModelSettings.test.ts — 设置·模型页（实体库）锁定：
// 折叠卡片列表（收起单行摘要 / 展开草稿编辑 + 显式保存/取消）+ 新建卡片（虚线第一条展开表单）
// + 换厂商 key 闸（保存时校验拦截）+ 被语义检索引用 profile 的编辑旁路
// （pendingEdit 重建确认）+ 创建校验 + 删除两步确认与守卫 inline。
// 视觉/语义引用选择已迁回设置-通用（GeneralSettings.test 覆盖）。
// Combobox 交互按仓内惯例从组件 emit 驱动（AgentForm.providers 同款）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ModelSettings from "../settings/ModelSettings.vue";
import Combobox from "../../components/common/Combobox.vue";
import EmbedSwitchOverlay from "../../components/common/EmbedSwitchOverlay.vue";
import { loadModelProfiles } from "../../composables/useModelProfiles";
import type { ModelProfile, ProviderInfo, UserPreferences } from "../../types";

const mockInvoke = vi.mocked(invoke);

// ===== 固定目录（useProviders/useModelProfiles 模块级缓存：本文件首测触发拉取） =====
const PROVIDERS: ProviderInfo[] = [
  { name: "glm", protocol: "openai", default_url: "https://open.bigmodel.cn/api/paas/v4", alt_urls: [], label: "智谱 GLM", note: null, requires_key: true, requires_base_url: false, key_url: "https://open.bigmodel.cn/keys", openai_url: "https://open.bigmodel.cn/api/paas/v4", hidden: false, models: ["glm-5.3-flash"] },
  { name: "openai", protocol: "openai", default_url: "https://api.openai.com/v1", alt_urls: [], label: "OpenAI", note: null, requires_key: true, requires_base_url: false, key_url: "https://platform.openai.com/api-keys", openai_url: "https://api.openai.com/v1", hidden: false, models: ["gpt-4o"] },
  { name: "ollama", protocol: "openai", default_url: "", alt_urls: [], label: "Ollama", note: null, requires_key: false, requires_base_url: true, key_url: null, openai_url: null, hidden: false, models: [] },
];

function profile(id: string, over: Partial<ModelProfile> = {}): ModelProfile {
  return {
    id, alias: id, provider: "glm", model: "glm-5.3-flash", base_url: null,
    sort_order: 0, created_at: "2026-09-08 00:00:00", updated_at: "2026-09-08 00:00:00",
    has_api_key: true,
    last_health: null, last_health_detail: null, last_health_at: null,
    ...over,
  };
}
/** DB UTC 串（N 分钟前）——健康胶囊相对时输入 */
function dbMinutesAgo(n: number): string {
  return new Date(Date.now() - n * 60000).toISOString().slice(0, 19).replace("T", " ");
}
const PROFILES: ModelProfile[] = [
  profile("mp-1", {
    alias: "智谱主力", provider: "glm", model: "glm-5.3-flash",
    last_health: "ok", last_health_at: dbMinutesAgo(3),
  }),
  profile("mp-2", {
    alias: "本地", provider: "ollama", model: "llava", base_url: "http://localhost:11434/v1",
    last_health: "quota", last_health_detail: "code:1113 无可用资源包", last_health_at: dbMinutesAgo(90),
  }),
  profile("mp-3", { alias: "备用" }),
];

/** 可变后端状态（update_model_profile 合并写回，模拟真实行） */
const db = { profiles: [...PROFILES] };
let prefs: UserPreferences = {};

function mockBackend() {
  mockInvoke.mockImplementation((async (cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case "get_preferences": return { ...prefs };
      case "list_providers": return PROVIDERS;
      case "list_model_profiles": return db.profiles.map((p) => ({ ...p }));
      case "update_model_profile": {
        const input = args!.input as ModelProfile & Record<string, unknown>;
        const hit = db.profiles.find((p) => p.id === input.id);
        if (!hit) throw new Error("profile not found");
        Object.assign(hit, input);
        return { ...hit };
      }
      case "set_preference": return undefined;
      case "test_embedding_config": return undefined;
      case "rebuild_all_embeddings": return { kbs: 1, chunks: 5 };
      case "delete_model_profile": {
        const id = args!.id as string;
        db.profiles = db.profiles.filter((p) => p.id !== id);
        return undefined;
      }
      default:
        return undefined;
    }
  }) as never);
}

async function mountPage() {
  const w = mount(ModelSettings);
  await flushPromises();
  return w;
}

/** 实体卡（过滤掉列表头的新建卡） */
function entityCards(w: ReturnType<typeof mount>) {
  return w.findAll(".profile-card").filter((c) => !c.classes().includes("new-card"));
}

/** 展开第 i 条实体卡 */
async function expandCard(w: ReturnType<typeof mount>, i: number) {
  await entityCards(w)[i].trigger("click");
  await flushPromises();
}

describe("ModelSettings 设置·模型页（实体库）", () => {
  beforeEach(async () => {
    mockInvoke.mockReset();
    db.profiles = PROFILES.map((p) => ({ ...p }));
    prefs = {};
    mockBackend();
    // 模块级缓存与 db 同步重置（前测的 update/reload 会污染缓存——如模型名已改）
    await loadModelProfiles(true);
  });

  it("挂载渲染：新建卡在列表头 + 实体卡双行（首行别名/引用数，次行厂商 tag 前置+模型+健康状态）", async () => {
    prefs = { vision_profile_ids: ["mp-1"], embedding_profile_id: "mp-1" };
    const w = await mountPage();
    const cards = w.findAll(".profile-card");
    expect(cards).toHaveLength(4); // 新建卡 + 3 实体

    expect(cards[0].classes()).toContain("new-card");
    expect(cards[0].text()).toContain("新建模型配置");

    const [a, b, c] = entityCards(w);
    // 首行：别名 + 引用次数（视觉 1 + 语义 1 = 2 处）；厂商 tag 已移次行
    const aTitle = a.find(".row-title");
    expect(aTitle.text()).toContain("智谱主力");
    expect(aTitle.find(".ref-count").text()).toBe("2 处引用");
    expect(aTitle.find(".ref-count").classes()).not.toContain("ref-count--none");
    expect(aTitle.find(".provider-badge").exists()).toBe(false);

    // 次行：厂商 tag 前置 + 模型名 + 健康胶囊（ok = 绿 + 相对时；端点/Key 徽标折叠态不上屏）
    const aSub = a.find(".row-sub");
    expect(aSub.find(".provider-badge").text()).toBe("智谱 GLM");
    expect(aSub.text()).toContain("glm-5.3-flash");
    const aHealth = aSub.find(".health-chip");
    expect(aHealth.text()).toContain("正常");
    expect(aHealth.text()).toMatch(/刚刚|\d+分钟前/);
    expect(aHealth.classes()).toContain("health-chip--success");
    expect(aHealth.attributes("title")).toContain("最后调用");
    expect(aSub.find(".card-url").exists()).toBe(false);
    expect(aSub.find(".key-badge").exists()).toBe(false);
    // 收起态无编辑字段
    expect(a.find("input").exists()).toBe(false);

    // quota = danger 胶囊 + hover title 带失败原文
    const bHealth = b.find(".health-chip");
    expect(bHealth.text()).toContain("额度耗尽");
    expect(bHealth.classes()).toContain("health-chip--danger");
    expect(bHealth.attributes("title")).toContain("无可用资源包");
    // 未被引用的实体：显示「未引用」最淡态
    expect(b.find(".ref-count").text()).toBe("未引用");
    expect(b.find(".ref-count").classes()).toContain("ref-count--none");

    // 从未调用：中性「未调用」，不带相对时
    const cHealth = c.find(".health-chip");
    expect(cHealth.text()).toContain("未调用");
    expect(cHealth.classes()).toContain("health-chip--neutral");
    expect(cHealth.text()).not.toMatch(/分钟前|小时前/);
  });

  it("点击展开：编辑字段出现（别名/厂商/模型/Key/端点），再点收起", async () => {
    const w = await mountPage();
    await expandCard(w, 0);
    const card = entityCards(w)[0];
    expect(card.find("input").exists()).toBe(true);
    expect((card.findAll('input[type="text"]')[0].element as HTMLInputElement).value).toBe("智谱主力");
    expect(card.find(".key-input").exists()).toBe(true);
    expect(card.findComponent(Combobox).exists()).toBe(true);

    await card.trigger("click");
    await flushPromises();
    expect(entityCards(w)[0].find("input").exists()).toBe(false);
  });

  it("Key 框三态：已存显示掩码圆点，点入转打点输入，空离开回掩码；眼睛切明文", async () => {
    const w = await mountPage();
    await expandCard(w, 0);
    const card = entityCards(w)[0];
    const key = card.find(".key-input");

    // 默认掩码态：圆点表示已存（密文永不回显），type=text 让圆点是字面字符
    expect((key.element as HTMLInputElement).value).toBe("••••••••••••••••");
    expect(key.attributes("type")).toBe("text");
    expect(key.classes()).toContain("key-masked");

    // 点入编辑：掩码让位，打点输入；眼睛出现（有草稿）
    await key.trigger("focus");
    expect(key.attributes("type")).toBe("password");
    expect((key.element as HTMLInputElement).value).toBe("");
    await key.setValue("sk-new");
    expect(key.attributes("type")).toBe("password");
    const eye = card.find(".key-eye");
    expect(eye.exists()).toBe(true);
    await eye.trigger("click");
    expect(key.attributes("type")).toBe("text");
    expect((key.element as HTMLInputElement).value).toBe("sk-new");

    // 空输入离开 → 回掩码、眼睛复位隐藏（明文不留在屏上）
    const key2 = card.find(".key-input");
    await key2.setValue("");
    await key2.trigger("blur");
    expect((key2.element as HTMLInputElement).value).toBe("••••••••••••••••");
    expect(card.find(".key-eye").exists()).toBe(false);
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "update_model_profile")).toBe(false);
  });

  it("显式保存：无变更保存禁用；改草稿后取消 → 回滚不落库；保存成功收起卡片", async () => {
    const w = await mountPage();
    await expandCard(w, 0);
    const card = entityCards(w)[0];
    const save = card.findAll("button").find((b) => b.text() === "保存")!;
    expect((save.element as HTMLButtonElement).disabled).toBe(true); // 无变更 → 禁用

    await card.findAll('input[type="text"]')[0].setValue("智谱备用");
    await flushPromises();
    expect((save.element as HTMLButtonElement).disabled).toBe(false);

    mockInvoke.mockClear();
    await card.findAll("button").find((b) => b.text() === "取消")!.trigger("click");
    await flushPromises();
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "update_model_profile")).toBe(false);
    expect(entityCards(w)[0].find("input").exists()).toBe(false); // 取消收起
    // 再展开：草稿已回滚为服务端值
    await expandCard(w, 0);
    expect((entityCards(w)[0].findAll('input[type="text"]')[0].element as HTMLInputElement).value).toBe("智谱主力");

    // 改别名 → 保存 → 整批提交成功后收起
    await entityCards(w)[0].findAll('input[type="text"]')[0].setValue("智谱备用");
    await entityCards(w)[0].findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("update_model_profile", {
      input: expect.objectContaining({ id: "mp-1", alias: "智谱备用", provider: "glm", base_url: null }),
    });
    expect(entityCards(w)[0].find("input").exists()).toBe(false);
  });

  it("换厂商 key 闸：切到需 Key 厂商未填 Key → 保存被校验拦截；补 Key 后整批提交", async () => {
    const w = await mountPage();
    await expandCard(w, 0);
    const card = entityCards(w)[0];
    mockInvoke.mockClear();

    // 切到 OpenAI（requires_key）且 Key 草稿空 → 点保存被拦截，零写入
    card.findAllComponents(Combobox)[0].vm.$emit("update:modelValue", "OpenAI");
    await flushPromises();
    await card.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(card.find(".eb-inline").text()).toContain("API Key");
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "update_model_profile")).toBe(false);
    expect(entityCards(w)[0].find("input").exists()).toBe(true); // 留开供修正

    // 补 Key → 保存 → 整批提交（alias + provider + model + base_url + api_key）
    await card.find(".key-input").setValue("sk-new");
    await card.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("update_model_profile", {
      input: expect.objectContaining({ id: "mp-1", provider: "openai", model: "gpt-4o", base_url: null, api_key: "sk-new" }),
    });
  });

  it("编辑被语义检索引用的 profile：弹 pendingEdit overlay，确认走 update → test → rebuild", async () => {
    prefs = { embedding_profile_id: "mp-1" };
    const w = await mountPage();
    await expandCard(w, 0);
    const card = entityCards(w)[0];
    mockInvoke.mockClear();

    // 模型 Combobox 改草稿 + 点保存 → 身份变化触发重建确认（update 未落）
    card.findAllComponents(Combobox)[1].vm.$emit("update:modelValue", "glm-4.5v");
    await flushPromises();
    await card.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(w.findComponent(EmbedSwitchOverlay).exists()).toBe(true);
    expect(w.findComponent(EmbedSwitchOverlay).text()).toContain("修改被语义检索引用");
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "update_model_profile")).toBe(false);

    await w.findAll("button").find((b) => b.text() === "确认并重建")!.trigger("click");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("update_model_profile", {
      input: expect.objectContaining({ id: "mp-1", model: "glm-4.5v" }),
    });
    expect(mockInvoke).toHaveBeenCalledWith("test_embedding_config", {
      provider: "", model: "", apiKey: "", baseUrl: null, profileId: "mp-1",
    });
    expect(mockInvoke).toHaveBeenCalledWith("rebuild_all_embeddings");
    expect(w.findComponent(EmbedSwitchOverlay).exists()).toBe(false);
  });

  it("pendingEdit 健康检查失败：配置已存但向量未重建（无自动回滚），overlay 留开文案明示", async () => {
    prefs = { embedding_profile_id: "mp-1" };
    const w = await mountPage();
    await expandCard(w, 0);
    entityCards(w)[0].findAllComponents(Combobox)[1].vm.$emit("update:modelValue", "glm-4.5v");
    await flushPromises();
    await entityCards(w)[0].findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();

    mockInvoke.mockImplementation((async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "test_embedding_config") throw new Error("模型不存在");
      if (cmd === "update_model_profile") {
        const input = args!.input as ModelProfile;
        const hit = db.profiles.find((p) => p.id === input.id);
        if (hit) Object.assign(hit, input);
        return { ...hit };
      }
      return (await mockBackendInvoke(cmd));
    }) as never);

    await w.findAll("button").find((b) => b.text() === "确认并重建")!.trigger("click");
    await flushPromises();

    expect(w.findComponent(EmbedSwitchOverlay).exists()).toBe(true); // 留开可修正
    expect(w.findComponent(EmbedSwitchOverlay).text()).toContain("配置已更新、向量未重建");
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "rebuild_all_embeddings")).toBe(false);
  });

  it("创建表单：点新建卡展开，空别名前端拦截（不发 create_model_profile）", async () => {
    const w = await mountPage();
    await w.findAll(".profile-card")[0].trigger("click"); // 新建卡
    await flushPromises();
    mockInvoke.mockClear();

    await w.findAll("button").find((b) => b.text() === "创建")!.trigger("click");
    await flushPromises();
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "create_model_profile")).toBe(false);
    expect(w.find(".test-fail-text").text()).toContain("别名必填");
  });

  it("删除两步确认：第一次点击武装成确认键，第二次执行；被引用时守卫拒绝 inline", async () => {
    const w = await mountPage();
    mockInvoke.mockImplementation((async (cmd: string) => {
      if (cmd === "delete_model_profile") {
        throw new Error("该配置仍被视觉读取引用：请先在「设置-通用-视觉读取」中移除引用后重试");
      }
      return (await mockBackendInvoke(cmd));
    }) as never);

    await expandCard(w, 0);
    const card = entityCards(w)[0];
    mockInvoke.mockClear();

    // 第一步：武装（零删除调用）
    await card.find('button[title*="删除此模型配置"]').trigger("click");
    await flushPromises();
    const confirmBtn = card.find(".delete-confirm-btn");
    expect(confirmBtn.exists()).toBe(true);
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "delete_model_profile")).toBe(false);

    // 第二步：执行 → 被引用守卫三段式拒绝，inline 指路（不自动解链）
    await confirmBtn.trigger("click");
    await flushPromises();
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "delete_model_profile")).toBe(true);
    expect(entityCards(w)[0].find(".eb-inline").text()).toContain("仍被视觉读取引用");
  });
});

/** 基线后端（mock 被覆写后复用：非覆写命令回落原实现） */
async function mockBackendInvoke(cmd: string): Promise<unknown> {
  switch (cmd) {
    case "get_preferences": return { ...prefs };
    case "list_providers": return PROVIDERS;
    case "list_model_profiles": return db.profiles.map((p) => ({ ...p }));
    case "set_preference": return undefined;
    case "rebuild_all_embeddings": return { kbs: 1, chunks: 5 };
    default: return undefined;
  }
}
