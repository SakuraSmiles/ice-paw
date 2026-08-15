// composables/useProviders.ts — Provider 目录共享加载
//
// 后端 PROVIDERS 注册表经 list_providers 命令下发（单一真相源）；
// AgentForm（下拉/校验规则）与 AgentSettings（徽标显示名）共用一份
// 模块级缓存，避免每个组件各自请求/各自硬编码。
//
// 失败降级：console.error + 空表——表单回落为纯手输（provider/model
// 都是自由文本），不因目录不可用而卡死配置流程。

import { ref } from "vue";
import { bridge } from "../api/bridge";
import type { ProviderInfo } from "../types";

// 模块级单例（跨组件、跨设置页/表单往返共享一次请求）
const providers = ref<ProviderInfo[]>([]);
const loaded = ref(false);
let loading: Promise<void> | null = null;

/** 加载 Provider 目录（幂等；force=true 强制刷新，如探测到目录变更时） */
export async function loadProviders(force = false): Promise<ProviderInfo[]> {
  if (loaded.value && !force) return providers.value;
  if (!loading || force) {
    loading = (async () => {
      try {
        providers.value = await bridge.providers.list();
        loaded.value = true;
      } catch (e) {
        // 降级为空表：表单回落纯手输，不引入第二份硬编码
        console.error("[useProviders] 加载 Provider 目录失败:", e);
        if (force) providers.value = [];
      } finally {
        loading = null;
      }
    })();
  }
  await loading;
  return providers.value;
}

/** 目录数据（ref，首次 loadProviders 前为空） */
export function useProviders() {
  return { providers, loadProviders };
}

/** provider 名 → 展示名；目录未加载/未收录（含旧数据自定义名）回退 name 原文 */
export function providerLabelOf(list: ProviderInfo[], name: string): string {
  return list.find((p) => p.name === name)?.label ?? name;
}
