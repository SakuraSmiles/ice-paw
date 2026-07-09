// 应用全局状态 Store：演示主题切换、侧边栏折叠等 UI 状态
// 与 counterStore 一样使用 Composition API 风格的 setup store
import { ref } from "vue";
import { defineStore } from "pinia";

/** 主题类型：明 / 暗 */
export type Theme = "light" | "dark";

/**
 * 应用全局 store
 * - state: theme（'light' | 'dark'）、sidebarCollapsed（boolean）
 * - actions: toggleTheme / toggleSidebar
 */
export const useAppStore = defineStore("app", () => {
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
