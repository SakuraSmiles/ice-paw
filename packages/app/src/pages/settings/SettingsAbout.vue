<script setup lang="ts">
import { ref, onMounted } from "vue";
import { ChevronDown, PawPrint } from "lucide-vue-next";

const appVersion = ref<string>("");
const showSysInfo = ref(false);

onMounted(async () => {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    appVersion.value = await getVersion();
  } catch {
    appVersion.value = "未知";
  }
});

const sysInfo = [
  { label: "运行时", value: "Tauri" },
  { label: "前端框架", value: "Vue 3" },
  { label: "后端语言", value: "Rust" },
];

function toggleSysInfo(): void { showSysInfo.value = !showSysInfo.value; }
</script>

<template>
  <div class="settings-about">
    <h2 class="section-title">关于</h2>
    <div class="app-info">
      <div class="app-logo"><PawPrint :size="28" class="logo-icon" aria-hidden="true" /></div>
      <div class="app-meta">
        <span class="app-name">IcePaw</span>
        <span class="app-version">v{{ appVersion || "..." }}</span>
      </div>
    </div>
    <div class="disclosure">
      <button class="disclosure-trigger" type="button" @click="toggleSysInfo">
        <span>系统信息</span>
        <ChevronDown :size="14" :class="['chevron', { open: showSysInfo }]" />
      </button>
      <div v-if="showSysInfo" class="disclosure-content">
        <div class="sys-info-row">
          <span class="sys-info-label">应用版本</span>
          <span class="sys-info-value">{{ appVersion || "未知" }}</span>
        </div>
        <div v-for="info in sysInfo" :key="info.label" class="sys-info-row">
          <span class="sys-info-label">{{ info.label }}</span>
          <span class="sys-info-value">{{ info.value }}</span>
        </div>
      </div>
    </div>
    <div class="links-section">
      <a class="link-item" href="https://github.com/nicepkg/ice-paw" target="_blank" rel="noopener noreferrer">GitHub</a>
      <a class="link-item" href="https://github.com/nicepkg/ice-paw#readme" target="_blank" rel="noopener noreferrer">文档</a>
      <a class="link-item" href="https://github.com/nicepkg/ice-paw/issues" target="_blank" rel="noopener noreferrer">问题反馈</a>
    </div>
    <div class="update-row">
      <button class="btn-secondary" type="button" disabled>检查更新</button>
      <span class="update-hint">当前已是最新版本</span>
    </div>
  </div>
</template>

<style scoped>
.settings-about { max-width: 640px; }
.section-title {
  font-size: var(--ip-text-heading-sm-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin-bottom: var(--ip-spacing-4);
}
.app-info { display: flex; align-items: center; gap: var(--ip-spacing-4); margin-bottom: var(--ip-spacing-6); }
.app-logo {
  width: 64px; height: 64px; display: flex; align-items: center; justify-content: center;
  background-color: var(--ip-color-bg-primary); border-radius: var(--ip-radius-lg);
  border: 1px solid var(--ip-color-border-default);
}
.logo-icon { font-size: 32px; line-height: 1; }
.app-meta { display: flex; flex-direction: column; gap: var(--ip-spacing-0_5); }
.app-name { font-size: var(--ip-text-heading-sm-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.app-version { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.disclosure {
  margin-bottom: var(--ip-spacing-6); border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md); overflow: hidden;
}
.disclosure-trigger {
  display: flex; align-items: center; justify-content: space-between; width: 100%;
  padding: var(--ip-spacing-3) var(--ip-spacing-4);
  font-size: var(--ip-text-body-size); font-family: inherit; font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary); background: transparent; border: none;
  cursor: pointer; transition: var(--ip-transition-colors);
}
.disclosure-trigger:hover { background-color: var(--ip-color-bg-hover); }
.chevron { transition: transform 0.2s ease; color: var(--ip-color-text-tertiary); }
.chevron.open { transform: rotate(180deg); }
.disclosure-content {
  padding: 0 var(--ip-spacing-4) var(--ip-spacing-3);
  border-top: 1px solid var(--ip-color-border-subtle);
}
.sys-info-row { display: flex; justify-content: space-between; padding: var(--ip-spacing-2) 0; font-size: var(--ip-text-body-sm-size); }
.sys-info-label { color: var(--ip-color-text-secondary); }
.sys-info-value { color: var(--ip-color-text-primary); font-family: var(--ip-font-mono); }
.links-section { display: flex; gap: var(--ip-spacing-4); margin-bottom: var(--ip-spacing-6); }
.link-item { font-size: var(--ip-text-body-size); color: var(--ip-color-accent-text); text-decoration: none; transition: var(--ip-transition-colors); }
.link-item:hover { text-decoration: underline; }
.update-row { display: flex; align-items: center; gap: var(--ip-spacing-3); }
.btn-secondary {
  display: inline-flex; align-items: center; justify-content: center;
  height: var(--ip-btn-h-sm); padding: 0 var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size); font-family: inherit; font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary); background: transparent;
  border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-btn-radius);
  cursor: pointer; transition: var(--ip-transition-colors);
}
.btn-secondary:hover:not(:disabled) {
  color: var(--ip-color-text-primary); border-color: var(--ip-color-border-hover);
  background-color: var(--ip-color-bg-hover);
}
.btn-secondary:disabled { opacity: 0.5; cursor: not-allowed; }
.update-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
</style>
