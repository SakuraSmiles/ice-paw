<script setup lang="ts">
// 全局 Toast 渲染层
//
// 职责：
//   - 从 useToast() 单例读取 toasts 列表并渲染
//   - 固定在视口右下角，逐条堆叠向上
//   - 点击单条可立即关闭
//
// 样式：
//   - 极简：圆角矩形 + 左侧色条 + 阴影
//   - 暗色模式通过 prefers-color-scheme: dark 切换

import { useToast } from "../../composables/useToast";

const toast = useToast();
</script>

<template>
  <div class="toast-container" aria-live="polite" aria-atomic="true">
    <transition-group name="toast">
      <div
        v-for="t in toast.toasts"
        :key="t.id"
        :class="['toast-item', `toast-${t.kind}`]"
        role="status"
        @click="toast.remove(t.id)"
      >
        <span class="toast-bar" aria-hidden="true" />
        <span class="toast-msg">{{ t.message }}</span>
      </div>
    </transition-group>
  </div>
</template>

<style scoped>
.toast-container {
  /* 固定在视口右下角 */
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 9999;
  display: flex;
  flex-direction: column;
  gap: 8px;
  pointer-events: none;
  max-width: calc(100vw - 32px);
}

.toast-item {
  /* 子项接收事件 */
  pointer-events: auto;
  display: flex;
  align-items: stretch;
  gap: 8px;
  min-width: 240px;
  max-width: 360px;
  padding: 10px 14px 10px 10px;
  background: var(--toast-bg, #ffffff);
  color: var(--toast-fg, #1a1a1a);
  border: 1px solid var(--toast-border, #e0e0e0);
  border-radius: 6px;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
  cursor: pointer;
  font-size: 14px;
  line-height: 1.4;
  user-select: none;
}

.toast-bar {
  /* 左侧色条 */
  flex: 0 0 3px;
  border-radius: 2px;
  background: var(--toast-bar, #4a90e2);
}

.toast-msg {
  flex: 1;
  word-break: break-word;
}

/* 类型配色 */
.toast-success {
  --toast-bar: #2ea043;
  --toast-border: #c8e6c9;
}
.toast-error {
  --toast-bar: #d93025;
  --toast-border: #ffcdd2;
}
.toast-info {
  --toast-bar: #1a73e8;
  --toast-border: #bbdefb;
}
.toast-warning {
  --toast-bar: #f29900;
  --toast-border: #ffe0b2;
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .toast-item {
    --toast-bg: #2a2a3a;
    --toast-fg: #f0f0f0;
    --toast-border: #3a3a4a;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  }
  .toast-success {
    --toast-border: #2e7d32;
  }
  .toast-error {
    --toast-border: #c62828;
  }
  .toast-info {
    --toast-border: #1565c0;
  }
  .toast-warning {
    --toast-border: #ef6c00;
  }
}

/* 进入/离开动画：自下而上滑入/滑出 */
.toast-enter-from {
  opacity: 0;
  transform: translateY(8px);
}
.toast-enter-active {
  transition: opacity 180ms ease, transform 180ms ease;
}
.toast-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
.toast-leave-active {
  transition: opacity 180ms ease, transform 180ms ease;
}
</style>