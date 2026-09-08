// useModelProfiles.test.ts — 模型配置共享加载层锁定：
// 模块级缓存（两次 load 一次请求）+ force 强刷（增删改后）+ 失败降级清缓存
// + 纯函数（profileById / resolveProfileChain 保序悬空跳过 / chainReferences）。
// ⚠️ 缓存用例有顺序依赖（模块级单例贯穿本文件），vitest 按文件内顺序执行。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  loadModelProfiles, useModelProfiles, profileById, resolveProfileChain, chainReferences,
} from "../useModelProfiles";
import type { ModelProfile } from "../../types";

const mockInvoke = vi.mocked(invoke);

function profile(id: string, alias = id): ModelProfile {
  return {
    id, alias, provider: "glm", model: "glm-5.3-flash", base_url: null,
    sort_order: 0, created_at: "2026-09-08 00:00:00", updated_at: "2026-09-08 00:00:00",
    has_api_key: true,
  };
}

describe("useModelProfiles 共享加载", () => {
  beforeEach(() => {
    mockInvoke.mockReset().mockResolvedValue([profile("mp-1"), profile("mp-2")] as never);
  });

  it("模块级缓存：两次 load 只发一次 list_model_profiles", async () => {
    const a = await loadModelProfiles();
    const b = await loadModelProfiles();
    expect(a).toHaveLength(2);
    expect(b).toBe(a); // 同一份数组（单例 ref）
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("list_model_profiles");
  });

  it("force=true 强刷（增删改后拿新表）", async () => {
    await loadModelProfiles();
    mockInvoke.mockClear();
    const next = await loadModelProfiles(true);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(next).toHaveLength(2);
  });

  it("useModelProfiles() 暴露同一单例 ref", () => {
    const { profiles } = useModelProfiles();
    expect(profiles.value).toHaveLength(2);
  });

  it("force 失败降级：清缓存为空表不残留旧值冒充新结果", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("db down") as never);
    const r = await loadModelProfiles(true);
    expect(r).toEqual([]);
    // 控制台降级日志（不卡调用方）
    expect(console.error).toHaveBeenCalled();
  });
});

describe("纯函数（引用解析）", () => {
  const list = [profile("mp-1", "智谱主力"), profile("mp-2", "本地")];

  it("profileById：命中返回实体，未命中 undefined", () => {
    expect(profileById(list, "mp-2")?.alias).toBe("本地");
    expect(profileById(list, "mp-x")).toBeUndefined();
  });

  it("resolveProfileChain：保序解析，悬空 id 跳过（展示层语义）", () => {
    const chain = resolveProfileChain(list, ["mp-2", "mp-gone", "mp-1"]);
    expect(chain.map((p) => p.id)).toEqual(["mp-2", "mp-1"]);
  });

  it("chainReferences：引用链成员判定", () => {
    expect(chainReferences(["mp-1", "mp-2"], "mp-1")).toBe(true);
    expect(chainReferences(["mp-1"], "mp-2")).toBe(false);
  });
});
