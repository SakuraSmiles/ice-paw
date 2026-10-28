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
withDefaults(
  defineProps<{ flow?: "overlay" | "inline"; side?: "left" | "right"; variant?: "line" | "tab" }>(),
  { flow: "overlay", side: "right", variant: "line" },
);
const emit = defineEmits<{ dragstart: [e: PointerEvent]; reset: [] }>();
</script>

<template>
  <div
    class="panel-resize-handle"
    :class="[variant === 'tab' ? 'tab' : flow === 'overlay' ? `overlay-${side}` : 'inline']"
    title="拖拽调整宽度 · 双击重置"
    @pointerdown="emit('dragstart', $event)"
    @dblclick="emit('reset')"
  />
</template>

<style scoped>
.panel-resize-handle {
  cursor: col-resize;
  z-index: var(--ip-z-badge);
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

/* ---- tab 变体（2026-10-28 三轮定版，侧栏悬浮面板形态）：「耳朵」凸舌——
   面板垂直中点、从面板右边线向外长出的小舌头。对齐关键（一轮错位的修正）：
   左缘贴齐面板边框**外缘**（right 偏移 = 宽度 + 边框 1px），面板竖线连续
   穿过不中断、耳朵上下描边与它形成干净 T 形接合——无双线、无悬空；右侧
   半圆收头（半径 = 半宽）。同底同描边 + xs 投影与面板一体。握感件 = 中央
   单竖条（tertiary / hover·拖拽点亮 primary 并伸长）；拖拽中耳朵整体
   primary-soft 高亮。事件面 = 耳朵本体（~12×40px，不再全高热区）。 */
.panel-resize-handle.tab {
  position: absolute;
  top: 50%;
  right: -13px; /* = 宽 12 + 边框 1：左缘恰贴面板边框外缘（对齐锚） */
  width: 12px;
  height: 40px;
  transform: translateY(-50%);
  border: 1px solid var(--ip-color-border-default);
  border-left: none; /* 左缘开口——竖线由面板边框延续，耳朵只添上下两横 */
  border-radius: 0 6px 6px 0; /* 半宽半径 = 右侧半圆收头 */
  background: var(--ip-color-bg-primary);
  box-shadow: var(--ip-shadow-xs);
  transition:
    background var(--ip-duration-fast) var(--ip-ease-out),
    border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.panel-resize-handle.tab::after {
  /* 握感单竖条（完整覆写 line 形态 ::after 基线——含 bottom/transform/opacity） */
  content: "";
  position: absolute;
  top: 50%;
  bottom: auto;
  left: 50%;
  width: 2px;
  height: 14px;
  transform: translate(-50%, -50%);
  border-radius: 1px;
  background: var(--ip-color-text-tertiary);
  opacity: 1;
  transition:
    background var(--ip-duration-fast) var(--ip-ease-out),
    height var(--ip-duration-fast) var(--ip-ease-out);
}
.panel-resize-handle.tab:hover::after,
.panel-resize-handle.tab:active::after {
  background: var(--ip-primary-600);
  height: 22px;
  opacity: 1;
  transform: translate(-50%, -50%);
}
.panel-resize-handle.tab:active {
  background: var(--ip-primary-soft-bg, rgba(var(--ip-primary-500-rgb), 0.08));
  border-color: var(--ip-primary-soft-border, rgba(var(--ip-primary-500-rgb), 0.25));
}
[data-theme="dark"] .panel-resize-handle.tab:hover::after,
[data-theme="dark"] .panel-resize-handle.tab:active::after {
  background: var(--ip-primary-400);
}
</style>
