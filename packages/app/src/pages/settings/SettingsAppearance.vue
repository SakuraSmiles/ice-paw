<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useSettingsStore } from "../../stores/settings";
import SettingRow from "../../components/common/SettingRow.vue";
import SegmentedControl from "../../components/common/SegmentedControl.vue";

const settingsStore = useSettingsStore();

onMounted(async () => {
  await settingsStore.load();
  applyTheme(settingsStore.prefs.theme ?? "system");
});

const themeOptions = [
  { label: "浅色", value: "light" },
  { label: "深色", value: "dark" },
  { label: "跟随系统", value: "system" },
];

const fontSizeOptions = [
  { label: "12px", value: 12 },
  { label: "14px", value: 14 },
  { label: "16px", value: 16 },
  { label: "18px", value: 18 },
];

const theme = computed<string>({
  get: () => settingsStore.prefs.theme ?? "system",
  set: (v: string) => {
    settingsStore.update("theme", v);
    applyTheme(v);
  },
});

const fontSize = computed<number>({
  get: () => settingsStore.prefs.font_size ?? 14,
  set: (v: number) => settingsStore.update("font_size", v),
});

function applyTheme(value: string): void {
  const el = document.documentElement;
  if (value === "system") {
    el.removeAttribute("data-theme");
  } else {
    el.setAttribute("data-theme", value);
  }
}

function onFontSizeChange(e: Event): void {
  fontSize.value = parseInt((e.target as HTMLInputElement).value, 10);
}
</script>

<template>
  <div class="settings-appearance">
    <h2 class="section-title">外观</h2>
    <SettingRow label="主题" description="应用的整体配色方案">
      <SegmentedControl v-model="theme" :options="themeOptions" />
    </SettingRow>
    <SettingRow label="代码块主题" description="消息中代码块的语法高亮配色">
      <select class="select-control" disabled>
        <option>GitHub</option>
      </select>
    </SettingRow>
    <SettingRow label="字体大小" description="消息正文的显示字号">
      <select class="select-control" :value="String(fontSize)" @change="onFontSizeChange">
        <option v-for="opt in fontSizeOptions" :key="opt.value" :value="String(opt.value)">
          {{ opt.label }}
        </option>
      </select>
    </SettingRow>
  </div>
</template>

<style scoped>
.settings-appearance { max-width: 640px; }
.section-title {
  font-size: var(--ip-text-heading-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin-bottom: var(--ip-spacing-4);
}
.select-control {
  min-width: 180px;
  height: var(--ip-btn-h-sm);
  padding: 0 var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size);
  font-family: inherit;
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: var(--ip-transition-colors);
  appearance: auto;
}
.select-control:hover:not(:disabled) { border-color: var(--ip-color-border-hover); }
.select-control:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
