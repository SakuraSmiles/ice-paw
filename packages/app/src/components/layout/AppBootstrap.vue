<script setup lang="ts">
// AppBootstrap.vue — REQ-XC-007 异步启动包装层
//
// 职责：
//   - 把 AppLayout 的「同步渲染 + onMounted 异步加载」转换为「async setup」形态，
//     让外层 <Suspense> 能进入 pending 状态 → 显示 SkeletonScreen fallback
//   - 在 setup() 中预先 await 关键 store 的初始化（agents / projects / conversations / settings），
//     这与 AppLayout.onMounted 中的逻辑等价但**前置到 setup 阶段**
//
// 与 AppLayout 的关系：
//   - AppLayout 保留原样（向后兼容、不破坏现有 onMounted 逻辑）
//   - 此组件在 setup 中先并行 await 4 个 store 的初始化，再渲染 AppLayout
//   - AppLayout 渲染时会跳过自身的 onMounted 中的对应初始化（store.ensureLoaded 幂等）
//
// 错误处理：
//   - 任一 store 初始化失败不阻断渲染（与 AppLayout 原逻辑一致：try/catch 吞掉）
//   - 真正的错误展示由外层 10s timeout 控制（超时即视为启动失败 → 渲染错误页）

import { useAgentsStore } from "../../stores/agents";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../../stores/projects";
import { useConversationsStore } from "../../stores/conversations";
import { useSettingsStore } from "../../stores/settings";
import { useBootstrap } from "../../composables/useBootstrap";
import AppLayout from "./AppLayout.vue";

// 启动期间至少给一个最小延迟（即使 4 个 store 都瞬时返回），让 fallback 有机会「闪一下」，
// 避免「骨架屏瞬间被替换」的视觉抖动。300ms 是个折中：够短不显得卡，够长能感知到骨架屏。
const MIN_BOOTSTRAP_DELAY_MS = 300;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// 并行启动所有 store（任何一个失败都不影响其他）
const agentsStore = useAgentsStore();
const projectsStore = useProjectsStore();
const conversationsStore = useConversationsStore();
const settingsStore = useSettingsStore();
const bootstrap = useBootstrap();

/**
 * 启动初始化（并行）
 *
 * - 失败被吞掉（与 AppLayout 原 onMounted 行为一致）
 * - 与 onMounted 行为等价但放在 setup() 中以便 Suspense 等到 resolve
 */
const initPromise = (async () => {
  const results = await Promise.allSettled([
    agentsStore.ensureLoaded(),
    projectsStore.loadAll().catch(() => {
      /* 失败不阻塞，使用默认项目 */
    }),
    conversationsStore.loadForProject(
      projectsStore.currentId || DEFAULT_PROJECT_ID,
    ).catch(() => {
      /* 失败不阻塞 */
    }),
    settingsStore.load().catch(() => {
      /* 失败不阻塞 */
    }),
  ]);

  // 至少 MIN_BOOTSTRAP_DELAY_MS 才 resolve，让 fallback 显示有感
  await sleep(MIN_BOOTSTRAP_DELAY_MS);

  // 仅 debug 日志，不抛错
  for (let i = 0; i < results.length; i++) {
    const r = results[i];
    if (r.status === "rejected") {
      const name = ["agents", "projects", "conversations", "settings"][i];
      console.warn(`[AppBootstrap] ${name} store 初始化失败：`, r.reason);
    }
  }
})();

// 注意：这里 await initPromise 会让 setup 变成 async setup，
// Vue 会把整个组件注册为 async component → 外层 <Suspense> 能进入 pending 状态。
await initPromise;

// 标记启动完成，让 AppLayout.onMounted 能跳过重复初始化
bootstrap.markBootstrapped();
</script>

<template>
  <!--
    bootstrap resolve 后才渲染 AppLayout（保证 skeleton 至少显示 MIN_BOOTSTRAP_DELAY_MS）
    AppLayout 自身的 onMounted 仍会跑（其内部 store 调用由于 ensureLoaded 幂等性不会重复触发网络）
  -->
  <AppLayout />
</template>