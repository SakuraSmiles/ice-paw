/**
 * REQ-CFG-005g：全局快捷键 composable。
 *
 * 职责：
 *   1. 在 App.vue 中调用 `installGlobalListener()` 注册全局 keydown 监听。
 *   2. 从 settingsStore.prefs.keyboard_shortcuts 读取绑定，解析失败回落到
 *      `DEFAULT_KEYBOARD_SHORTCUTS`。
 *   3. 匹配时通过回调执行业务逻辑。
 */

import { computed, ref, watch, toValue, type MaybeRefOrGetter, type Ref } from "vue";
import type { UserPreferences } from "../types";
import {
  DEFAULT_KEYBOARD_SHORTCUTS,
  SHORTCUT_ACTION_IDS,
} from "../utils/defaults";

// ============================================================================
// 类型
// ============================================================================

/** 快捷键绑定映射：actionId → 组合键字符串 */
export type KeyboardShortcuts = Record<string, string>;

// ============================================================================
// 快捷键解析工具
// ============================================================================

/**
 * 将 KeyboardEvent 转为规范化组合键字符串（如 "Ctrl+K"、"Cmd+,"、"Enter"）。
 */
export function normalizeKeyEvent(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.metaKey) parts.push("Cmd");
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");

  let key = e.key;
  if (key.length === 1) {
    key = key.toUpperCase();
  }
  parts.push(key);
  return parts.join("+");
}

/**
 * 解析 UserPreferences.keyboard_shortcuts，失败回落到默认值。
 */
export function parseKeyboardShortcuts(raw: unknown): KeyboardShortcuts {
  if (
    raw !== null &&
    raw !== undefined &&
    typeof raw === "object" &&
    !Array.isArray(raw)
  ) {
    const obj = raw as Record<string, unknown>;
    const result: KeyboardShortcuts = {};
    for (const id of SHORTCUT_ACTION_IDS) {
      const v = obj[id];
      if (typeof v === "string" && v.length > 0) {
        result[id] = v;
      } else {
        result[id] = DEFAULT_KEYBOARD_SHORTCUTS[id];
      }
    }
    return result;
  }
  return { ...DEFAULT_KEYBOARD_SHORTCUTS };
}

/**
 * 匹配规范化快捷键字符串。
 * 支持 Cmd/Ctrl 互替（macOS 用 Cmd，Windows/Linux 用 Ctrl）。
 */
export function matchShortcut(normalized: string, binding: string): boolean {
  if (normalized === binding) return true;
  if (normalized.replace("Ctrl", "Cmd") === binding) return true;
  if (normalized.replace("Cmd", "Ctrl") === binding) return true;
  return false;
}

// ============================================================================
// composable
// ============================================================================

/**
 * REQ-CFG-005g：安装全局 keydown 监听器。
 *
 * @param prefsSource 响应式的 prefs 来源（settingsStore.prefs / `() => settingsStore.prefs`
 *                    / ComputedRef 等皆可）。
 *                    Pinia store 会自动解包内部 ref，因此传入 getter 是最稳妥的形式。
 * @param onAction 匹配成功时的回调
 */
export function useKeyboard(
  prefsSource: MaybeRefOrGetter<UserPreferences>,
  onAction: (action: string) => void,
): { shortcuts: Ref<KeyboardShortcuts>; cleanup: () => void } {
  const shortcuts = ref<KeyboardShortcuts>({
    ...DEFAULT_KEYBOARD_SHORTCUTS,
  });

  // 取 keyboard_shortcuts 的派生 ref，兼容 Ref / ComputedRef / getter / 普通值。
  const keyboardShortcutsRef = computed(
    () => toValue(prefsSource).keyboard_shortcuts,
  );

  function refresh(): void {
    shortcuts.value = parseKeyboardShortcuts(keyboardShortcutsRef.value);
  }

  // 当 prefs.keyboard_shortcuts 变化时自动刷新
  watch(keyboardShortcutsRef, () => refresh(), { immediate: true });

  function onKeydown(e: KeyboardEvent): void {
    // 忽略输入框内的快捷键（除非有 Ctrl/Cmd 修饰）
    const tag = (e.target as HTMLElement)?.tagName;
    if (
      tag === "INPUT" ||
      tag === "TEXTAREA" ||
      (e.target as HTMLElement)?.isContentEditable
    ) {
      if (!e.ctrlKey && !e.metaKey) return;
    }

    const normalized = normalizeKeyEvent(e);
    for (const [action, binding] of Object.entries(shortcuts.value)) {
      if (matchShortcut(normalized, binding)) {
        e.preventDefault();
        e.stopPropagation();
        onAction(action);
        return;
      }
    }
  }

  window.addEventListener("keydown", onKeydown);

  const cleanup = () => {
    window.removeEventListener("keydown", onKeydown);
  };

  return { shortcuts, cleanup };
}

export default useKeyboard;
