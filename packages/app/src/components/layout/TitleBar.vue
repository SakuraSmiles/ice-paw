<!-- TitleBar — 自绘窗口栏（感官升级批 3 · L3 完全一体感）
     Windows-first：decorations:false 后系统栏退役，本组件承接拖拽/三键。
     ⚠️ 取舍已拍板（2026-10-26）：Win11 Snap Layouts 悬停最大化按钮的布局
     弹层丢失（DWM 层能力，自绘按钮无法触发）；Aero Snap（Win+方向键/拖到
     边缘）不受影响。Alt+Space 系统菜单仍可用。
     macOS 交通灯不支持（产品线 Windows NSIS 单端）。
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
      <button class="tb-btn" type="button" aria-label="最小化" @click="onMinimize">
        <Minus :size="14" :stroke-width="1.75" />
      </button>
      <button
        class="tb-btn"
        type="button"
        :aria-label="maximized ? '还原' : '最大化'"
        @click="onToggleMaximize"
      >
        <Copy v-if="maximized" :size="13" :stroke-width="1.75" />
        <Square v-else :size="12" :stroke-width="1.75" />
      </button>
      <button class="tb-btn tb-close" type="button" aria-label="关闭" @click="onClose">
        <X :size="15" :stroke-width="1.75" />
      </button>
    </div>
  </header>
</template>

<style scoped>
/* 栏体 = 侧栏顶缘的视觉延续（同底色），全宽压在 Sidebar + 内容区之上，
   呈「框」而非「横带」——一体感的关键。高度 36px 贴 Discord/Slack 惯例。
   左侧恒空（去字标拍板）：按钮区右贴，其余整条为拖拽区。 */
.titlebar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  height: 36px;
  flex-shrink: 0;
  background: var(--ip-color-bg-base); /* 窗口基底语义别名（=bg-primary；2026-10-28 收口：基底位统一走别名） */
  /* 底缘横线退役（2026-10-28 悬浮面板化）：TitleBar 属窗口基底（L0），侧栏
     以缝隙+圆角+投影浮于其下——基底保持连续无分割线 */
  user-select: none;
}

/* 失焦降对比（不消失——窗口控制恒可见是平台惯例） */
.titlebar:not(.focused) .tb-btn { color: var(--ip-color-text-tertiary); }

.tb-actions {
  display: flex;
  height: 100%;
}

/* 三键命中区 46px 宽贴 Windows 惯例；hover 微底色（关闭键 danger 惯例红）。
   drag-region 属性只挂在栏体——按钮区不挂，点击不会被拖拽吞掉。 */
.tb-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 46px;
  height: 100%;
  border: none;
  background: transparent;
  color: var(--ip-color-text-secondary);
  cursor: default; /* 窗口控制不是链接语义，default 比 pointer 贴平台惯例 */
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out, ease);
}

.tb-btn:hover {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}

.tb-btn:active {
  background: var(--ip-color-border-default);
}

.tb-close:hover {
  background: #e81123; /* Windows 关闭键惯例红（语义层 danger 令牌为此场景的例外位） */
  color: #fff;
}

.tb-btn:focus-visible {
  outline: 2px solid var(--ip-color-border-focus, var(--ip-primary-400));
  outline-offset: -2px;
}
</style>
