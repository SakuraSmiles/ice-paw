// 全局 Toast 轻量提示
//
// 设计：
//   - 模块单例（模块级 reactive 状态），不依赖 Pinia
//   - 任意处 `import { useToast }` 拿到同一份状态
//   - 默认 3 秒自动消失，可传入 duration 覆盖（毫秒；0 表示不自动消失）
//   - 暴露 success / error / info / warning 四种预设
//
// 用法：
//   const toast = useToast();
//   toast.error('保存失败');
//   toast.success('已创建', { duration: 2000 });

import { reactive } from "vue";

// ============================================================================
// 类型定义
// ============================================================================

/** Toast 类型（影响配色） */
export type ToastKind = "success" | "error" | "info" | "warning";

/** 单条 Toast */
export interface ToastItem {
  /** 唯一 ID（用于 v-for key 与手动移除） */
  id: number;
  /** 提示文本 */
  message: string;
  /** 类型 */
  kind: ToastKind;
  /** 自动消失时长（毫秒）；0 表示不自动消失 */
  duration: number;
}

/** add 调用选项 */
export interface ToastOptions {
  /** 自动消失时长（毫秒），默认 3000；0 表示不自动消失 */
  duration?: number;
}

// ============================================================================
// 模块级状态（单例）
// ============================================================================

/** Toast 列表（响应式） */
const toasts = reactive<ToastItem[]>([]);

/** 下一个分配的 ID */
let nextId = 1;

// ============================================================================
// 内部方法
// ============================================================================

/**
 * 新增一条 Toast
 * @param message 提示文本
 * @param kind 类型
 * @param options 选项
 * @returns 分配的 ID
 */
function add(message: string, kind: ToastKind, options?: ToastOptions): number {
  const id = nextId++;
  const duration = options?.duration ?? 3000;
  toasts.push({ id, message, kind, duration });

  if (duration > 0) {
    // 使用 setTimeout 自动移除
    window.setTimeout(() => {
      remove(id);
    }, duration);
  }
  return id;
}

/**
 * 移除指定 ID 的 Toast
 * @param id Toast ID
 */
function remove(id: number): void {
  const idx = toasts.findIndex((t) => t.id === id);
  if (idx >= 0) {
    toasts.splice(idx, 1);
  }
}

/**
 * 清空全部 Toast
 */
function clear(): void {
  toasts.splice(0, toasts.length);
}

// ============================================================================
// composable 导出
// ============================================================================

/**
 * 全局 Toast composable。
 *
 * 返回对象包含 toasts（只读引用）+ 四个预设方法 + remove/clear。
 */
export function useToast() {
  return {
    /** 当前所有 Toast（只读引用，供 Toast.vue 渲染） */
    toasts,
    /** 新增一条 Toast */
    add,
    /** 移除指定 Toast */
    remove,
    /** 清空全部 Toast */
    clear,
    /** 成功提示（绿色） */
    success(message: string, options?: ToastOptions): number {
      return add(message, "success", options);
    },
    /** 错误提示（红色） */
    error(message: string, options?: ToastOptions): number {
      return add(message, "error", options);
    },
    /** 信息提示（蓝色） */
    info(message: string, options?: ToastOptions): number {
      return add(message, "info", options);
    },
    /** 警告提示（黄色） */
    warning(message: string, options?: ToastOptions): number {
      return add(message, "warning", options);
    },
  };
}

export default useToast;