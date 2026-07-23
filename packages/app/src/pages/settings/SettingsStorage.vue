<script setup lang="ts">
import { ref, onMounted } from "vue";
import { Copy } from "lucide-vue-next";
import SettingRow from "../../components/common/SettingRow.vue";

const appDataDir = ref<string>("");

onMounted(async () => {
  try {
    const { appDataDir: getAppDataDir } = await import("@tauri-apps/api/path");
    appDataDir.value = await getAppDataDir();
  } catch {
    appDataDir.value = "获取失败";
  }
});

async function copyDataDir(): Promise<void> {
  try { await navigator.clipboard.writeText(appDataDir.value); } catch { /* noop */ }
}

async function openDataDir(): Promise<void> {
  try {
    const { openPath } = await import("@tauri-apps/plugin-opener");
    await openPath(appDataDir.value);
  } catch { /* noop */ }
}
</script>

<template>
  <div class="settings-storage">
    <h2 class="section-title">存储</h2>
    <SettingRow label="数据位置" description="应用数据存储目录">
      <div class="dir-row">
        <span class="dir-text">{{ appDataDir || "加载中..." }}</span>
        <button class="btn-icon-sm" type="button" title="复制路径" @click="copyDataDir"><Copy :size="14" /></button>
      </div>
    </SettingRow>
    <SettingRow label="打开数据文件夹" description="在系统文件管理器中打开数据目录">
      <button class="btn-secondary" type="button" :disabled="!appDataDir" @click="openDataDir">打开</button>
    </SettingRow>
  </div>
</template>

<style scoped>
.settings-storage { max-width: 640px; }
.section-title {
  font-size: var(--ip-text-heading-sm-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin-bottom: var(--ip-spacing-4);
}
.dir-row { display: flex; align-items: center; gap: var(--ip-spacing-2); }
.dir-text {
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary);
  font-family: var(--ip-font-mono); max-width: 260px;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.btn-icon-sm {
  display: inline-flex; align-items: center; justify-content: center;
  width: 28px; height: 28px; border: none; border-radius: var(--ip-radius-sm);
  background: transparent; cursor: pointer; transition: var(--ip-transition-colors); font-size: 14px;
}
.btn-icon-sm:hover { background-color: var(--ip-color-bg-hover); }
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
</style>
