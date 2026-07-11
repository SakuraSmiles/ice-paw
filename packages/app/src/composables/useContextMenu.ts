// 全局右键菜单状态管理
//
// 设计：
//   - 模块单例（模块级 reactive 状态），不依赖 Pinia
//   - 任意处 import { useContextMenu } 拿到同一份状态
//   - 外部点击 / Esc 关闭由 ContextMenu.vue 组件的 document 监听器负责
//
// 用法：
//   const ctx = useContextMenu();
//   ctx.openMenu(event.clientX, event.clientY, [
//     { label: '重命名', handler: () => doRename() },
//     { label: '删除', danger: true, handler: () => doDelete() },
//   ]);

import { reactive } from "vue";

// ============================================================================
// 类型定义
// ============================================================================

/** 右键菜单单项配置 */
export interface ContextMenuItem {
  /** 显示文本 */
  label: string;
  /** 是否为危险操作（红色高亮） */
  danger?: boolean;
  /** 点击该项时执行的回调（外层负责关闭菜单） */
  handler: () => void;
}

/** 菜单位置（视口坐标） */
export interface ContextMenuPosition {
  /** 视口 X 坐标 */
  x: number;
  /** 视口 Y 坐标 */
  y: number;
}

/** 全局右键菜单状态 */
interface ContextMenuState {
  /** 是否显示 */
  visible: boolean;
  /** 菜单位置 */
  position: ContextMenuPosition;
  /** 菜单项列表 */
  items: ContextMenuItem[];
}

// ============================================================================
// 模块级状态（单例）
// ============================================================================

/** 全局状态（响应式单例） */
const state = reactive<ContextMenuState>({
  visible: false,
  position: { x: 0, y: 0 },
  items: [],
});

// ============================================================================
// composable 导出
// ============================================================================

/**
 * 全局右键菜单 composable。
 *
 * 返回对象包含：
 *   - state       响应式状态（visible / position / items），由 ContextMenu.vue 订阅
 *   - openMenu    打开菜单
 *   - closeMenu   关闭菜单并清空项
 */
export function useContextMenu() {
  /**
   * 打开菜单。
   * @param x     视口 X 坐标（一般取 event.clientX）
   * @param y     视口 Y 坐标（一般取 event.clientY）
   * @param items 菜单项列表（按显示顺序）
   */
  function openMenu(x: number, y: number, items: ContextMenuItem[]): void {
    state.position.x = x;
    state.position.y = y;
    state.items = items;
    state.visible = true;
  }

  /** 关闭菜单并清空项 */
  function closeMenu(): void {
    state.visible = false;
    state.items = [];
  }

  return {
    /** 当前状态（只读引用，供模板订阅） */
    state,
    /** 打开菜单 */
    openMenu,
    /** 关闭菜单 */
    closeMenu,
  };
}

export default useContextMenu;