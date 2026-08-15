// composables/useResizablePanel.ts
// 可调面板宽度（UX #2 规范化）：宽度状态 + 拖拽手势 + localStorage 记忆的唯一实现。
//
// 从 TrajectoryView 手搓版抽出 generalize：任何可调宽面板 = 本 composable
// （状态/手势/持久化）+ PanelResizeHandle（视觉把手），不再各写各的 mousemove。
//
// 约定：
// - localStorage key = `icepaw-panel-{key}`（对齐 useTheme 的 icepaw-* 命名）
// - 拖拽中零写盘，pointerup 才落一次（避免拖动过程写盘风暴）
// - clamp 双保险：min/max 静态界限 + 视口 50% 动态上限（防小窗口拖爆）
// - 拖拽期间全局 col-resize 光标 + 禁文本选中（把手在 window 上收 move/up）

import { ref, onBeforeUnmount } from "vue";

/** 面板配置（全 px） */
export interface ResizablePanelOptions {
  /** 持久化 key 后缀（完整 key = `icepaw-panel-{key}`） */
  key: string;
  /** 默认宽度（双击把手重置到此值） */
  default: number;
  /** 最小宽度 */
  min: number;
  /** 最大宽度 */
  max: number;
  /**
   * 拖拽方向：+1 = 指针右移变宽（把手在面板右缘，如侧栏）；
   * -1 = 指针左移变宽（把手在左缘，如右侧检查器）
   */
  dir: 1 | -1;
}

/**
 * 宽度钳制（纯函数，供单测）：先夹 min/max 静态界，再夹视口 50% 动态上限。
 * 视口上限不会低于 min（极端小窗口下面板保底可用，宁可超出半屏）。
 */
export function clampPanelWidth(v: number, min: number, max: number, viewport: number): number {
  const cap = Math.max(min, Math.min(max, Math.floor(viewport / 2)));
  return Math.min(cap, Math.max(min, Math.round(v)));
}

export function useResizablePanel(opts: ResizablePanelOptions) {
  const storageKey = `icepaw-panel-${opts.key}`;
  const width = ref(opts.default);

  // 启动恢复：坏值（NaN/空）落回默认；恢复值也过一遍钳制
  //（窗口可能比上次保存时更小）
  const saved = Number.parseInt(localStorage.getItem(storageKey) ?? "", 10);
  if (Number.isFinite(saved)) {
    width.value = clampPanelWidth(saved, opts.min, opts.max, window.innerWidth);
  }

  const dragging = ref(false);

  /** 把手 pointerdown 入口（PanelResizeHandle 转发）。非主键忽略。 */
  function startDrag(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    dragging.value = true;
    const startX = e.clientX;
    const startW = width.value;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const onMove = (mv: PointerEvent) => {
      width.value = clampPanelWidth(
        startW + opts.dir * (mv.clientX - startX),
        opts.min,
        opts.max,
        window.innerWidth,
      );
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      dragging.value = false;
      // 松手才落盘（拖动中零写盘）
      localStorage.setItem(storageKey, String(width.value));
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }

  /** 双击把手：回默认宽并清掉持久化（下次启动也是默认） */
  function reset() {
    width.value = opts.default;
    localStorage.removeItem(storageKey);
  }

  // 组件卸载时拖拽可能仍在进行（极端：拖拽中切路由）——补一次清场，
  // 防止全局 cursor/userSelect 残留。监听由 onUp 自摘，无需重复 remove。
  onBeforeUnmount(() => {
    if (dragging.value) {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      dragging.value = false;
    }
  });

  return { width, dragging, startDrag, reset };
}
