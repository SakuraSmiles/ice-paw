<script setup lang="ts">
// 拖拽上传 overlay 组件
//
// 当文件拖入聊天区域时显示的全屏半透明遮罩，
// 提示用户释放鼠标以添加图片。
// 纯展示组件，不处理任何逻辑（由 ChatPage 控制 show/hide）。

import { ImagePlus } from "lucide-vue-next";
</script>

<template>
  <div class="drag-overlay">
    <div class="drag-overlay-content">
      <ImagePlus :size="48" aria-hidden="true" />
      <p class="drag-overlay-text">释放以添加图片</p>
    </div>
  </div>
</template>

<style scoped>
.drag-overlay {
  position: absolute;
  inset: 0;
  z-index: 100;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.4);
  pointer-events: none;
}

.drag-overlay-content {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-8);
  border: 2px dashed var(--ip-color-border-focus, #3b82f6);
  border-radius: var(--ip-radius-lg);
  background: var(--ip-color-bg-primary, rgba(255, 255, 255, 0.95));
  color: var(--ip-color-text-secondary);
}

.drag-overlay-text {
  margin: 0;
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}
</style>
