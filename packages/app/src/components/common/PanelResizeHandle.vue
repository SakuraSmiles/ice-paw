<!--
  PanelResizeHandle — 可调面板边缘拖拽把手（UX #2 规范化统一视觉）

  隐形热区形态（用户拍板）：默认完全不可见，hover/拖拽时浮现一条 2px 主题色
  细亮线。双击 = 重置默认宽度。手势/状态一律走 useResizablePanel——本组件
  只管视觉与事件转发，不含任何宽度逻辑。

  Props:
  - flow = "overlay"（默认）：绝对定位贴面板边缘，面板容器需 position:relative；
    `side` 决定贴左/右缘（把手悬在面板边框外 3px）
  - flow = "inline"：作为 flex 兄弟项占 6px（左右面板内容间的间隙，如轨迹页
    检查器与表格之间），无需 relative 容器
  Emits:
  - dragstart(pointerdown)：父级接 useResizablePanel().startDrag
  - reset(dblclick)：父级接 useResizablePanel().reset
-->
<script setup lang="ts">
withDefaults(defineProps<{ flow?: "overlay" | "inline"; side?: "left" | "right" }>(), {
  flow: "overlay",
  side: "right",
});
const emit = defineEmits<{ dragstart: [e: PointerEvent]; reset: [] }>();
</script>

<template>
  <div
    class="panel-resize-handle"
    :class="[flow === 'overlay' ? `overlay-${side}` : 'inline']"
    title="拖拽调整宽度 · 双击重置"
    @pointerdown="emit('dragstart', $event)"
    @dblclick="emit('reset')"
  />
</template>

<style scoped>
.panel-resize-handle {
  cursor: col-resize;
  z-index: 5;
  touch-action: none; /* 拖拽优先于滚动/触摸手势 */
}
/* overlay：贴面板边缘悬空（热区横跨边框两侧） */
.panel-resize-handle.overlay-left,
.panel-resize-handle.overlay-right {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 7px;
}
.panel-resize-handle.overlay-left { left: -3px; }
.panel-resize-handle.overlay-right { right: -3px; }
/* inline：布局内的窄间隙项（面板与内容之间） */
.panel-resize-handle.inline {
  position: relative;
  flex-shrink: 0;
  width: 7px;
  align-self: stretch;
}

/* 隐形热区：默认不可见，hover/拖拽中（:active 在按住期间持续）浮现细亮线 */
.panel-resize-handle::after {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 50%;
  width: 2px;
  transform: translateX(-50%) scaleY(0.3);
  border-radius: 1px;
  background: var(--ip-primary-500);
  opacity: 0;
  transition:
    opacity var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out);
}
.panel-resize-handle:hover::after,
.panel-resize-handle:active::after {
  opacity: 0.65;
  transform: translateX(-50%) scaleY(1);
}
/* 暗色下提亮一档保可见 */
[data-theme="dark"] .panel-resize-handle:hover::after,
[data-theme="dark"] .panel-resize-handle:active::after {
  background: var(--ip-primary-400);
  opacity: 0.8;
}
</style>
