// composables/useTheme.ts
// 暗色模式：本地持久化 + 跟随系统偏好 + View Transitions API 扩散切换 +
// 同步 Tauri 原生窗口主题（ImmersiveDarkMode）。
//
// 从 Sidebar.vue 抽出：零 store 耦合，纯 DOM/localStorage/Tauri window 副作用，
// 便于在侧栏以外复用（如设置页）。

import { ref, onMounted } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { bridge } from "../api/bridge";

export function useTheme() {
  const isDark = ref(false);

  function applyTheme(dark: boolean) {
    isDark.value = dark;
    document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
    localStorage.setItem("icepaw-theme", dark ? "dark" : "light");
    // 主题标记双写（主题底色秒窗，2026-10-27）：文件是 Rust 在窗口创建前唯一
    // 可读的真相源（localStorage 那时不可达）；失败静默——下次切换重写自愈。
    bridge.preferences.setThemeMarker(dark).catch(() => {});
    // 同步原生窗口主题：让 Windows 标题栏（含最小化/最大化/关闭按钮）跟随应用主题，
    // 走 Tauri 的 ImmersiveDarkMode（setTheme），与上面 DOM 的 data-theme 保持一致。
    // 非 Tauri 环境（纯 web 预览）会 reject，静默忽略。
    getCurrentWindow().setTheme(dark ? "dark" : "light").catch(() => {});
  }

  function toggleTheme() {
    const newDark = !isDark.value;

    // View Transitions API：从按钮位置扩散切换（WebView2 Chromium 111+ 支持）
    if (document.startViewTransition) {
      const btn = document.querySelector(".btn-theme-toggle");
      const rect = btn?.getBoundingClientRect();
      const x = rect ? (rect.left + rect.width / 2) / window.innerWidth * 100 : 50;
      const y = rect ? (rect.top + rect.height / 2) / window.innerHeight * 100 : 50;

      // 用 CSS 变量传递圆心位置，纯 CSS keyframe 驱动动画
      document.documentElement.style.setProperty("--theme-reveal-x", x + "%");
      document.documentElement.style.setProperty("--theme-reveal-y", y + "%");

      const transition = document.startViewTransition(() => {
        applyTheme(newDark);
      });

      transition.finished.finally(() => {
        document.documentElement.style.removeProperty("--theme-reveal-x");
        document.documentElement.style.removeProperty("--theme-reveal-y");
      });
    } else {
      applyTheme(newDark);
    }
  }

  onMounted(() => {
    const saved = localStorage.getItem("icepaw-theme");
    // 首启默认浅色（2026-10-27 拍板，四处对齐：此处 / splash anti-FOUC 脚本 /
    // Rust theme_window_bg marker 缺失分支 / tauri.conf backgroundColor 默认值）。
    // saved 存在即 applyTheme——挂载自愈顺带补写主题标记文件（旧版本升级用户
    // localStorage 有值但 marker 缺失，首启纯色窗曾错色一次，此后对齐）。
    applyTheme(saved ? saved === "dark" : false);
  });

  return { isDark, toggleTheme };
}
