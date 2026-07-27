// 用户偏好默认值（前端镜像）
//
// REQ-CFG-001 / 001a：与 Rust 侧 `src-tauri/src/db/models.rs::default_preferences()`
// 严格保持同步。增删字段必须前后端同步修改。
//
// 设计说明：
// - 所有字段都用非空值（去掉 Optional），因为前端通过 `bridge.preferences.getWithDefaults()`
//   拿到的就是完整 struct；但保留本表供以下场景使用：
//   1. 旧接口 `bridge.preferences.get()`（缺失字段）兜底；
//   2. ChatInput.vue 等子组件读取 `send_shortcut` 默认行为；
//   3. settings store 初始化（load 未完成时显示）。
//
// ⚠️ 同步流程：修改本文件后必须同步修改 `src-tauri/src/db/models.rs::default_preferences()`
// 和 `KNOWN_KEYS` 列表。

import type { UserPreferences } from "../types";

// ============================================================================
// REQ-CFG-005：默认快捷键映射
// ============================================================================

/** 快捷键功能 ID → 中文显示名 */
export const SHORTCUT_ACTION_LABELS: Record<string, string> = {
  sendMessage: "发送消息",
  newConversation: "新建会话",
  toggleSidebar: "切换侧边栏",
  search: "搜索",
  openSettings: "打开设置",
};

/** 所有可配置的快捷键功能 ID（有序） */
export const SHORTCUT_ACTION_IDS: string[] = [
  "sendMessage",
  "newConversation",
  "toggleSidebar",
  "search",
  "openSettings",
];

/** 默认快捷键绑定（与 Rust 侧保持同步） */
export const DEFAULT_KEYBOARD_SHORTCUTS: Record<string, string> = {
  sendMessage: "Enter",
  newConversation: "Ctrl+N",
  toggleSidebar: "Ctrl+B",
  search: "Ctrl+K",
  openSettings: "Ctrl+,",
};

/**
 * REQ-CFG-001 / 001a：用户偏好的默认值。
 *
 * 与 Rust 侧 `default_preferences()` 一一对应。
 *
 * 注意：使用宽松类型 (`UserPreferences`) 而非 `Required<>`，避免 TypeScript 在
 * 可选属性 vs. literal union（如 `send_shortcut: "enter" | "ctrl_enter"`）上的
 * 推断冲突。前端所有调用点都走 nullish 兜底 (`?? DEFAULT.x`)，类型安全由调用方保证。
 */
export const DEFAULT_PREFERENCES: UserPreferences = {
  default_agent_id: null, // null = 「自动（第一个）」，见 SettingsGeneral.vue
  default_template_id: null, // null = 「无模板」
  on_startup: "chat",
  language: "zh-CN",
  theme: "system",
  code_theme: "auto",
  font_size: 14,
  default_provider: null, // null = 「未配置」
  send_shortcut: "enter", // "enter" | "ctrl_enter"
  auto_scroll: true,
  auto_render: true,
  auto_timestamp: true,
  keyboard_shortcuts: { ...DEFAULT_KEYBOARD_SHORTCUTS },
};

/**
 * REQ-CFG-001：把任意 UserPreferences 补齐缺失字段为默认值。
 *
 * 用于：
 * - `bridge.preferences.get()` 老接口（缺失字段为 undefined）；
 * - settings store 初始化（load 还未完成时）。
 */
export function fillPreferences(
  prefs: Partial<UserPreferences> | null | undefined,
): UserPreferences {
  const p = prefs ?? {};
  return {
    default_agent_id: p.default_agent_id ?? DEFAULT_PREFERENCES.default_agent_id,
    default_template_id:
      p.default_template_id ?? DEFAULT_PREFERENCES.default_template_id,
    on_startup: p.on_startup ?? DEFAULT_PREFERENCES.on_startup,
    language: p.language ?? DEFAULT_PREFERENCES.language,
    theme: p.theme ?? DEFAULT_PREFERENCES.theme,
    code_theme: p.code_theme ?? DEFAULT_PREFERENCES.code_theme,
    font_size: p.font_size ?? DEFAULT_PREFERENCES.font_size,
    default_provider: p.default_provider ?? DEFAULT_PREFERENCES.default_provider,
    send_shortcut: p.send_shortcut ?? DEFAULT_PREFERENCES.send_shortcut,
    auto_scroll: p.auto_scroll ?? DEFAULT_PREFERENCES.auto_scroll,
    auto_render: p.auto_render ?? DEFAULT_PREFERENCES.auto_render,
    auto_timestamp: p.auto_timestamp ?? DEFAULT_PREFERENCES.auto_timestamp,
    keyboard_shortcuts:
      p.keyboard_shortcuts ?? DEFAULT_PREFERENCES.keyboard_shortcuts,
  };
}