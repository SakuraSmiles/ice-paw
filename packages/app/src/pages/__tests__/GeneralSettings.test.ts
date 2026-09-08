// GeneralSettings.test.ts — 通用设置页锁定：本地环境卡 + 视觉读取（profile 引用链）
// + 语义检索（单引用 + 切换重建 overlay 三步流）。实体在「设置-模型」维护，
// 本页管引用（vision_profile_ids / embedding_profile_id 的 Some=权威语义）。
// Combobox 交互按仓内惯例从组件 emit 驱动（AgentForm.providers 同款）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import GeneralSettings from "../settings/GeneralSettings.vue";
import Combobox from "../../components/common/Combobox.vue";
import EmbedSwitchOverlay from "../../components/common/EmbedSwitchOverlay.vue";
import { loadModelProfiles } from "../../composables/useModelProfiles";
import type { ModelProfile, ProviderInfo, UserPreferences } from "../../types";

const mockInvoke = vi.mocked(invoke);

// ===== 固定目录（useProviders/useModelProfiles 模块级缓存：本文件首测触发拉取） =====
const PROVIDERS: ProviderInfo[] = [
  { name: "glm", protocol: "openai", default_url: "https://open.bigmodel.cn/api/paas/v4", alt_urls: [], label: "智谱 GLM", note: null, requires_key: true, requires_base_url: false, key_url: "https://open.bigmodel.cn/keys", openai_url: "https://open.bigmodel.cn/api/paas/v4", hidden: false, models: ["glm-5.3-flash"] },
];

function profile(id: string, over: Partial<ModelProfile> = {}): ModelProfile {
  return {
    id, alias: id, provider: "glm", model: "glm-5.3-flash", base_url: null,
    sort_order: 0, created_at: "2026-09-08 00:00:00", updated_at: "2026-09-08 00:00:00",
    has_api_key: true, ...over,
  };
}
const PROFILES: ModelProfile[] = [
  profile("mp-1", { alias: "智谱主力" }),
  profile("mp-2", { alias: "本地", model: "glm-4.5v" }),
];

let prefs: UserPreferences = {};

function mockBackend() {
  mockInvoke.mockImplementation((async (cmd: string) => {
    switch (cmd) {
      case "get_preferences": return { ...prefs };
      case "list_providers": return PROVIDERS;
      case "list_model_profiles": return PROFILES.map((p) => ({ ...p }));
      case "get_data_dir": return "C:/Users/dev/AppData/Roaming/com.icepaw.app";
      case "set_preference": return undefined;
      case "test_embedding_config": return undefined;
      case "rebuild_all_embeddings": return { kbs: 1, chunks: 5 };
      default: return undefined;
    }
  }) as never);
}

async function mountPage() {
  const w = mount(GeneralSettings);
  await flushPromises();
  return w;
}

/** 三卡按序返回 [卡1 本地环境, 卡2 视觉读取, 卡3 语义检索] */
function cards(w: ReturnType<typeof mount>) {
  const list = w.findAll("section.settings-card");
  expect(list).toHaveLength(3);
  return list;
}

describe("GeneralSettings 通用设置页（视觉/语义引用）", () => {
  beforeEach(async () => {
    mockInvoke.mockReset();
    prefs = {};
    mockBackend();
    // 模块级缓存与固定目录同步重置（前测 force reload 可能带入污染）
    await loadModelProfiles(true);
  });

  it("挂载渲染：本地环境 + 视觉链行数来自 prefs + 语义检索引用回显", async () => {
    prefs = { vision_profile_ids: ["mp-1", "mp-2"], embedding_profile_id: "mp-2" };
    const w = await mountPage();
    const [, b, c] = cards(w);

    expect(b.text()).toContain("视觉读取");
    expect(b.findAll(".vision-entry")).toHaveLength(2);
    expect(b.findAll(".vision-tag")[0].text()).toBe("主模型");
    expect(b.findAll(".vision-tag")[1].text()).toBe("降级 1");

    expect(c.text()).toContain("语义检索");
    expect((c.findComponent(Combobox).vm.$props as { modelValue: string }).modelValue).toBe("mp-2");
  });

  it("语义检索未启用：选择即存 embedding_profile_id（无 overlay、无重建）", async () => {
    prefs = {}; // 未启用（无旧向量，无需重建）
    const w = await mountPage();
    cards(w)[2].findComponent(Combobox).vm.$emit("update:modelValue", "mp-1");
    await flushPromises();

    expect(mockInvoke).toHaveBeenCalledWith("set_preference", { key: "embedding_profile_id", value: '"mp-1"' });
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "rebuild_all_embeddings")).toBe(false);
    expect(w.findComponent(EmbedSwitchOverlay).exists()).toBe(false);
  });

  it("已启用换引用：overlay 二次确认 → test → save → rebuild 三步流", async () => {
    prefs = { embedding_profile_id: "mp-2" };
    const w = await mountPage();
    cards(w)[2].findComponent(Combobox).vm.$emit("update:modelValue", "mp-1");
    await flushPromises();

    // 确认前零写入
    expect(w.findComponent(EmbedSwitchOverlay).exists()).toBe(true);
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "set_preference")).toBe(false);

    await w.findAll("button").find((b) => b.text() === "确认并重建")!.trigger("click");
    await flushPromises();

    // 健康检查（profileId 腿）→ 存引用 → 全量重建
    expect(mockInvoke).toHaveBeenCalledWith("test_embedding_config", {
      provider: "", model: "", apiKey: "", baseUrl: null, profileId: "mp-1",
    });
    expect(mockInvoke).toHaveBeenCalledWith("set_preference", { key: "embedding_profile_id", value: '"mp-1"' });
    expect(mockInvoke).toHaveBeenCalledWith("rebuild_all_embeddings");
    // overlay 关闭 + 结果留屏（成功信息不随 overlay 蒸发）
    expect(w.findComponent(EmbedSwitchOverlay).exists()).toBe(false);
    expect(cards(w)[2].text()).toContain("已切换并重建 5 个向量");
  });

  it("健康检查失败：未切换、原配置保留（overlay 留开，错误文案明示）", async () => {
    prefs = { embedding_profile_id: "mp-2" };
    const w = await mountPage();
    mockInvoke.mockImplementation((async (cmd: string) => {
      if (cmd === "test_embedding_config") throw new Error("401 key 无效");
      return (await mockBackendInvoke(cmd));
    }) as never);

    cards(w)[2].findComponent(Combobox).vm.$emit("update:modelValue", "mp-1");
    await flushPromises();
    await w.findAll("button").find((b) => b.text() === "确认并重建")!.trigger("click");
    await flushPromises();

    expect(w.findComponent(EmbedSwitchOverlay).exists()).toBe(true); // overlay 留开
    expect(w.findComponent(EmbedSwitchOverlay).text()).toContain("未切换，原配置保留");
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "set_preference")).toBe(false);
  });

  it("视觉链操作：上移即存新序、添加降级为本地占位不落库", async () => {
    prefs = { vision_profile_ids: ["mp-1", "mp-2"] };
    const w = await mountPage();
    const b = cards(w)[1];
    mockInvoke.mockClear();

    // 上移降级条目（ChevronUp = 每 entry 第一个 chain-ops 按钮）
    await b.findAll(".vision-entry")[1].findAll("button")[0].trigger("click");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("set_preference", {
      key: "vision_profile_ids", value: '["mp-2","mp-1"]',
    });

    // 添加降级 = 本地空占位（选择后才落库）
    mockInvoke.mockClear();
    await b.findAll("button").find((x) => x.text().includes("添加降级模型"))!.trigger("click");
    await flushPromises();
    expect(b.findAll(".vision-entry")).toHaveLength(3);
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === "set_preference")).toBe(false);
  });
});

/** 基线后端（mock 被覆写后复用：非覆写命令回落原实现） */
async function mockBackendInvoke(cmd: string): Promise<unknown> {
  switch (cmd) {
    case "get_preferences": return { ...prefs };
    case "list_providers": return PROVIDERS;
    case "list_model_profiles": return PROFILES.map((p) => ({ ...p }));
    case "get_data_dir": return "C:/Users/dev/AppData/Roaming/com.icepaw.app";
    case "set_preference": return undefined;
    case "rebuild_all_embeddings": return { kbs: 1, chunks: 5 };
    default: return undefined;
  }
}
