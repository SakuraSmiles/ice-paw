// UI 状态 Store：主题与侧边栏折叠等纯前端展示状态
// 主题与侧边栏折叠属于纯 UI 状态，暂不持久化，后续接入 settings 页再补
import { ref } from "vue";
import { defineStore } from "pinia";

/** 主题类型：明 / 暗 */
export type Theme = "light" | "dark";

/**
 * UI store
 * - state: theme（'light' | 'dark'）、sidebarCollapsed（boolean）
 * - actions: toggleTheme / toggleSidebar
 */
export const useUiStore = defineStore("ui", () => {
  // ===== state =====
  const theme = ref<Theme>("light");
  const sidebarCollapsed = ref<boolean>(false);

  // ===== actions =====

  /** 切换明暗主题：light <-> dark */
  function toggleTheme() {
    theme.value = theme.value === "light" ? "dark" : "light";
  }

  /** 切换侧边栏折叠状态 */
  function toggleSidebar() {
    sidebarCollapsed.value = !sidebarCollapsed.value;
  }

  return {
    theme,
    sidebarCollapsed,
    toggleTheme,
    toggleSidebar,
  };
});
