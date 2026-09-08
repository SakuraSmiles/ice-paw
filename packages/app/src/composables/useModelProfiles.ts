// composables/useModelProfiles.ts — 模型配置（ModelProfile）共享加载
//
// useProviders 模块级缓存同款：设置-模型页（三卡共用一份列表）与未来的
// Phase 2 AgentForm（profile 选择器）共享一次请求；增删改后 force 刷新。
//
// 失败降级：console.error + 空表——引用链选择器回落为空态提示，不卡设置页。

import { computed, ref } from "vue";
import { bridge } from "../api/bridge";
import type { ModelProfile } from "../types";

// 模块级单例（跨组件共享）
const profiles = ref<ModelProfile[]>([]);
const loaded = ref(false);
const loading = ref(false);

/** 加载模型配置列表（幂等；force=true 强制刷新——增删改/轮换 key 后） */
export async function loadModelProfiles(force = false): Promise<ModelProfile[]> {
  if (loaded.value && !force) return profiles.value;
  if (loading.value && !force) return profiles.value;
  loading.value = true;
  try {
    profiles.value = await bridge.modelProfiles.list();
    loaded.value = true;
  } catch (e) {
    // 降级为空表；force 清缓存避免残留旧值冒充新结果
    console.error("[useModelProfiles] 加载模型配置失败:", e);
    if (force) profiles.value = [];
  } finally {
    loading.value = false;
  }
  return profiles.value;
}

export function useModelProfiles() {
  return { profiles, loading, loadModelProfiles };
}

/** id → profile（未收录返回 undefined，如已删除的悬空引用） */
export function profileById(list: ModelProfile[], id: string): ModelProfile | undefined {
  return list.find((p) => p.id === id);
}

/** 引用链批量解析为 profile 列表（保序；悬空 id 跳过——展示层用，不在此 warn） */
export function resolveProfileChain(list: ModelProfile[], ids: string[]): ModelProfile[] {
  return ids
    .map((id) => profileById(list, id))
    .filter((p): p is ModelProfile => Boolean(p));
}

/** 引用链是否含指定 id（卡 A 编辑守卫的判据——改 provider/model 弹重建提示用） */
export function chainReferences(chainIds: string[], profileId: string): boolean {
  return chainIds.includes(profileId);
}

/** 供模板直接用的派生：当前引用链（悬空引用过滤后） */
export function useResolvedChain(ids: () => string[]) {
  return computed(() => resolveProfileChain(profiles.value, ids()));
}
