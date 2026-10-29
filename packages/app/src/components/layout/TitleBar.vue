<!-- TitleBar — 自绘窗口栏（感官升级批 3 · L3 完全一体感）
     Windows-first：decorations:false 后系统栏退役，本组件承接拖拽/三键。
     ⚠️ 取舍已拍板（2026-10-26）：Win11 Snap Layouts 悬停最大化按钮的布局
     弹层丢失（DWM 层能力，自绘按钮无法触发）；Aero Snap（Win+方向键/拖到
     边缘）不受影响。Alt+Space 系统菜单仍可用。
     macOS 原生交通灯不支持（Windows NSIS 单端），但三键采用 mac 交通灯
     视觉（2026-10-29 拍板：12px 彩点 + hover 微符号；键序仍 Windows 惯例）。
     栏体只罩内容区（侧栏 2026-10-29 顶到顶部）。
-->
<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, Copy, X } from "@lucide/vue";

/** 最大化态（图标切换 max/restore）；失焦态（栏体与按钮降对比，激活窗口
     语义的廉价信号）。初始 -1 = 未知（首次 onResized/isMaximized 回填前
     用 maximize 图标占位，避免闪换）。 */
const maximized = ref<boolean | null>(null);
const focused = ref(true);

// happy-dom 测试环境无 Tauri runtime：getCurrentWindow 抛错须静默（ChatHeader
// setTitle 同款防御）；三个 unlisten 存根保证 onUnmounted 无条件可调。
let unlistenResize: () => void = () => {};
let unlistenFocus: () => void = () => {};

async function refreshMaximized() {
  try {
    maximized.value = await getCurrentWindow().isMaximized();
  } catch {
    /* 测试环境 */
  }
}

function onMinimize() {
  getCurrentWindow().minimize().catch(() => {});
}
function onToggleMaximize() {
  getCurrentWindow().toggleMaximize().catch(() => {});
}
function onClose() {
  getCurrentWindow().close().catch(() => {});
}

onMounted(async () => {
  await refreshMaximized();
  try {
    unlistenResize = await getCurrentWindow().onResized(() => void refreshMaximized());
    unlistenFocus = await getCurrentWindow().onFocusChanged(({ payload }) => {
      focused.value = payload;
    });
  } catch {
    /* 测试环境 */
  }
});
onUnmounted(() => {
  unlistenResize();
  unlistenFocus();
});
</script>

<template>
  <!-- 双击最大化由 Tauri 注入脚本内建处理（drag.js：detail===2 →
       internal_toggle_maximize，需 capabilities 放行）——组件勿再挂 @dblclick，
       双触发会「最大化后立即还原」。 -->
  <header
    class="titlebar"
    :class="{ focused }"
    data-tauri-drag-region
    aria-label="窗口栏"
  >
    <!-- 左侧无内容（2026-10-27 用户拍板去字标）：整条栏体空白区皆拖拽区 -->
    <div class="tb-actions" role="group" aria-label="窗口控制">
      <button class="tb-btn tb-min" type="button" aria-label="最小化" @click="onMinimize">
        <Minus :size="9" :stroke-width="3" />
      </button>
      <button
        class="tb-btn tb-max"
        type="button"
        :aria-label="maximized ? '还原' : '最大化'"
        @click="onToggleMaximize"
      >
        <Copy v-if="maximized" :size="9" :stroke-width="3" />
        <Square v-else :size="9" :stroke-width="3" />
      </button>
      <button class="tb-btn tb-close" type="button" aria-label="关闭" @click="onClose">
        <X :size="9" :stroke-width="3" />
      </button>
    </div>
  </header>
</template>

<style scoped>
/* 栏体只罩内容区（2026-10-29 侧栏顶到顶部拍板）：侧栏全高后 TitleBar 不再
   整宽压顶。高度 36px；左侧恒空（去字标拍板）：按钮区右贴，其余整条为
   拖拽区（侧栏 header 空白区兼任拖拽——窗口拖拽面不缩水）。 */
.titlebar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  height: 36px;
  flex-shrink: 0;
  background: var(--ip-color-bg-base); /* 窗口基底语义别名（=bg-primary；2026-10-28 收口：基底位统一走别名） */
  user-select: none;
}

/* ---- 三键 = mac 交通灯风格（2026-10-29 拍板）：12px 彩色圆点、静止无符号、
   hover 浮现微型符号（lucide 图标常驻 DOM 仅视觉隐藏——测试锁图标切换）。
   键序保持 Windows 惯例（最小化/最大化/关闭 = 黄/绿/红），颜色即语义跨平台
   无歧义；命中区 20px（圆点 12px 视觉 + 呼吸边距）。失焦态圆点退灰
   （mac 惯例的激活语义）。drag-region 只挂栏体——按钮区不挂。 */
.tb-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 100%;
  padding-right: 10px; /* 圆点组离窗口右缘的呼吸位 */
}

.tb-btn {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: default; /* 窗口控制不是链接语义，default 比 pointer 贴平台惯例 */
}

/* 圆点本体 */
.tb-btn::before {
  content: "";
  position: absolute;
  inset: 4px; /* 20px 命中区 → 12px 圆点 */
  border-radius: 50%;
  transition: filter var(--ip-duration-fast) var(--ip-ease-out, ease);
}
.tb-min::before  { background: #febc2e; box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.12); }
.tb-max::before  { background: #28c840; box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.12); }
.tb-close::before { background: #ff5f57; box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.12); }

/* 微型符号：静止隐藏、hover/聚焦浮现（lucide 常驻 DOM——aria/键盘可达不断） */
.tb-btn :deep(svg) {
  position: relative;
  z-index: 1;
  width: 9px;
  height: 9px;
  stroke-width: 3;
  color: rgba(0, 0, 0, 0.55);
  opacity: 0;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out, ease);
}
.tb-btn:hover :deep(svg),
.tb-btn:focus-visible :deep(svg) {
  opacity: 1;
}

/* 失焦：圆点退灰（激活窗口语义；符号不浮现） */
.titlebar:not(.focused) .tb-btn::before {
  background: var(--ip-color-bg-tertiary);
  box-shadow: inset 0 0 0 1px var(--ip-color-border-default);
}
.titlebar:not(.focused) .tb-btn :deep(svg) { opacity: 0; }

.tb-btn:focus-visible {
  outline: 2px solid var(--ip-color-border-focus, var(--ip-primary-400));
  outline-offset: -1px;
  border-radius: 50%;
}
</style>
