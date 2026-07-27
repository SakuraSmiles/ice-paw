<script setup lang="ts">
// App.vue — 应用根组件
//
// REQ-XC-007：骨架屏 + 加载超时
//   - 用 <Suspense> 包裹 AppBootstrap（异步 setup → 进入 pending → fallback=SkeletonScreen）
//   - 10s 超时后切换 `loadTimedOut = true`，渲染错误页（提供「重新加载」按钮）
//   - 用户点击「重新加载」会调 window.location.reload() 重新启动整个应用
//
// 三种状态：
//   1. loadTimedOut = false → <Suspense> 显示 SkeletonScreen（pending）或 AppLayout（resolved）
//   2. loadTimedOut = true  → 错误页（独立组件）
//
// 设计要点：
//   - 计时器在 onMounted 启动，组件卸载时清理（onUnmounted），避免内存泄漏
//   - 计时器只在顶层触发一次；用户手动 reload 后重新计时
//   - 超时时长提取为常量 LOAD_TIMEOUT_MS（10s），便于将来调整
//   - 错误页保留最小可读性与可操作性：「错误标题 + 错误描述 + 重新加载按钮」

import { onMounted, onUnmounted, ref } from "vue";
import SkeletonScreen from "./components/skeleton/SkeletonScreen.vue";
import AppBootstrap from "./components/layout/AppBootstrap.vue";

// ============================================================================
// 常量
// ============================================================================

/** REQ-XC-007：加载超时时长（10 秒） */
const LOAD_TIMEOUT_MS = 10_000;

// ============================================================================
// 状态
// ============================================================================

/** 是否已超时（true → 渲染错误页，false → 渲染 Suspense） */
const loadTimedOut = ref(false);

let timeoutHandle: ReturnType<typeof setTimeout> | null = null;

onMounted(() => {
  timeoutHandle = setTimeout(() => {
    loadTimedOut.value = true;
    console.warn(`[App.vue] 加载超过 ${LOAD_TIMEOUT_MS / 1000}s，切换到错误页`);
  }, LOAD_TIMEOUT_MS);
});

onUnmounted(() => {
  if (timeoutHandle !== null) {
    clearTimeout(timeoutHandle);
    timeoutHandle = null;
  }
});

/** 错误页的「重新加载」按钮：调 window.location.reload() */
function handleReload(): void {
  window.location.reload();
}
</script>

<template>
  <!--
    分支 1：超时 → 错误页
    分支 2：未超时 → Suspense（fallback=SkeletonScreen，default=AppBootstrap）
  -->
  <div v-if="loadTimedOut" class="app-load-error" role="alert">
    <div class="app-load-error-card">
      <div class="app-load-error-icon" aria-hidden="true">⚠️</div>
      <h1 class="app-load-error-title">加载超时</h1>
      <p class="app-load-error-desc">
        应用初始化超过了 {{ LOAD_TIMEOUT_MS / 1000 }} 秒仍未完成。<br />
        可能是网络异常或后端服务未启动。
      </p>
      <button
        type="button"
        class="app-load-error-btn"
        @click="handleReload"
      >
        重新加载
      </button>
    </div>
  </div>

  <Suspense v-else>
    <template #default>
      <AppBootstrap />
    </template>
    <template #fallback>
      <SkeletonScreen />
    </template>
  </Suspense>
</template>

<style scoped>
/* =========================================================================
 * 错误页样式（与设计 token 对齐）
 * ========================================================================= */
.app-load-error {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100vh;
  width: 100vw;
  background: var(--ip-color-bg-primary);
  padding: 24px;
}

.app-load-error-card {
  max-width: 420px;
  width: 100%;
  padding: 32px 28px;
  border-radius: var(--ip-radius-lg, 12px);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.04);
  text-align: center;
}

.app-load-error-icon {
  font-size: 48px;
  line-height: 1;
  margin-bottom: 16px;
}

.app-load-error-title {
  font-size: var(--ip-text-h2-size, 22px);
  line-height: var(--ip-text-h2-lh, 28px);
  font-weight: var(--ip-font-weight-semibold, 600);
  color: var(--ip-color-text-primary);
  margin: 0 0 12px;
}

.app-load-error-desc {
  font-size: var(--ip-text-body-sm-size, 13px);
  line-height: var(--ip-text-body-sm-lh, 20px);
  color: var(--ip-color-text-secondary);
  margin: 0 0 24px;
}

.app-load-error-btn {
  display: inline-block;
  padding: 10px 24px;
  font-size: var(--ip-text-body-sm-size, 13px);
  font-weight: var(--ip-font-weight-medium, 500);
  color: var(--ip-color-text-on-primary, #ffffff);
  background: var(--ip-primary-600, #2563eb);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-md, 6px);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-base, 200ms) var(--ip-ease-out),
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.app-load-error-btn:hover {
  background: var(--ip-primary-700, #1d4ed8);
}

.app-load-error-btn:active {
  transform: scale(0.98);
}

.app-load-error-btn:focus-visible {
  outline: 2px solid var(--ip-color-border-focus);
  outline-offset: 2px;
}
</style>