// 聊天输入文本草稿 composable
//
// 职责：
//   - 拥有 draft 响应式状态（v-model 友好）
//   - 32KB（UTF-8 字节）上限检测：
//     * watch 监听 draft，500ms debounce 后截断并提示
//     * 暴露 truncateIfNeeded() 供粘贴场景立即调用
//   - 截断采用二分定位最大可保留字符数，保留 UTF-8 字符完整性
//   - Toast 1 秒冷却避免连续弹窗
//
// 设计理由：
//   - 32KB 是 REQ-CHAT-001 规定的硬上限（不是 maxlength 字符数）
//   - debounce 让快速连打不会被实时截断打断光标
//   - 粘贴场景由调用方在 nextTick 中显式触发 truncateIfNeeded，无需等 debounce

import { ref, watch, type Ref } from "vue";
import { useToast } from "./useToast";

/** 文本输入字节上限（UTF-8 编码），REQ-CHAT-001 */
export const MAX_TEXT_BYTES = 32 * 1024;

/** 超限截断的 debounce 窗口（ms），REQ-CHAT-001 */
const TRUNCATE_DEBOUNCE_MS = 500;

/** Toast 重复抑制窗口（ms） */
const TOAST_COOLDOWN_MS = 1000;

/**
 * 计算字符串的 UTF-8 字节数
 * （比 `.length` 准确：表情 / 中文 不会被低估）
 */
function byteLength(s: string): number {
  return new Blob([s]).size;
}

/**
 * 把字符串截断到 ≤ MAX_TEXT_BYTES 字节
 * 用二分定位最大可保留的字符数，避免破坏 UTF-8 字符
 */
function truncateToLimit(s: string): string {
  if (byteLength(s) <= MAX_TEXT_BYTES) return s;
  let lo = 0;
  let hi = s.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1;
    if (byteLength(s.slice(0, mid)) <= MAX_TEXT_BYTES) {
      lo = mid;
    } else {
      hi = mid - 1;
    }
  }
  return s.slice(0, lo);
}

/**
 * 聊天输入 composable
 *
 * @example
 *   const { draft, truncateIfNeeded, clearDraft, MAX_TEXT_BYTES } = useChatInput();
 *   // <textarea v-model="draft" @paste="onPaste" />
 *   function onPaste(e) {
 *     // 文本粘贴后立即检测，无需等 500ms debounce
 *     void nextTick(() => truncateIfNeeded());
 *   }
 */
export function useChatInput() {
  const toast = useToast();
  const draft = ref<string>("");

  /** debounce 定时器 */
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  /** 上次 toast 时间戳，用于冷却 */
  let lastToastAt = 0;

  /** 弹一次「输入已达到上限(32KB)」toast（带冷却） */
  function showTruncateToast(): void {
    const now = Date.now();
    if (now - lastToastAt < TOAST_COOLDOWN_MS) return;
    lastToastAt = now;
    toast.warning("输入已达到上限(32KB)");
  }

  /**
   * 检查 draft 是否超限，若超限则截断并弹 toast
   * @returns 是否发生了截断
   */
  function truncateIfNeeded(): boolean {
    const cur = draft.value;
    if (byteLength(cur) <= MAX_TEXT_BYTES) return false;
    draft.value = truncateToLimit(cur);
    showTruncateToast();
    return true;
  }

  // watch draft：500ms debounce 后做截断
  // - 快速连打：每次按键都重置定时器，不打断光标
  // - 停下来：500ms 后自动截断超限内容 + toast
  watch(draft, () => {
    if (debounceTimer) {
      clearTimeout(debounceTimer);
    }
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      truncateIfNeeded();
    }, TRUNCATE_DEBOUNCE_MS);
  });

  /** 清空 draft 并取消待执行的 debounce 截断 */
  function clearDraft(): void {
    if (debounceTimer) {
      clearTimeout(debounceTimer);
      debounceTimer = null;
    }
    draft.value = "";
  }

  return {
    draft: draft as Ref<string>,
    truncateIfNeeded,
    clearDraft,
    MAX_TEXT_BYTES,
  };
}

export default useChatInput;
