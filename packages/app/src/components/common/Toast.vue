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
  background: var(--ip-color-bg-elevated);
  color: var(--ip-color-text-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md);
  cursor: pointer;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  user-select: none;
}

.toast-bar {
  /* 左侧色条 */
  flex: 0 0 3px;
  border-radius: 2px;
  background: var(--toast-bar, var(--ip-primary-500));
}

.toast-msg {
  flex: 1;
  word-break: break-word;
}

/* 类型配色 */
.toast-success {
  --toast-bar: var(--ip-success-base);
  --toast-border: var(--ip-success-border);
}
.toast-error {
  --toast-bar: var(--ip-danger-base);
  --toast-border: var(--ip-danger-border);
}
.toast-info {
  --toast-bar: var(--ip-info-base);
  --toast-border: var(--ip-info-border);
}
.toast-warning {
  --toast-bar: var(--ip-warning-base);
  --toast-border: var(--ip-warning-border);
}

/* 暗色模式（依赖 @ice-paw/ui 已注入 [data-theme=dark] / media 自动适配） */

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