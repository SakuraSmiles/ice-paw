<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useAgentsStore } from "../../stores/agents";
import { useTemplatesStore } from "../../stores/templates";
import { useSettingsStore } from "../../stores/settings";
import SettingRow from "../../components/common/SettingRow.vue";
import SegmentedControl from "../../components/common/SegmentedControl.vue";

const agentsStore = useAgentsStore();
const templatesStore = useTemplatesStore();
const settingsStore = useSettingsStore();

onMounted(async () => {
  await agentsStore.ensureLoaded();
  await templatesStore.ensureLoaded();
  await settingsStore.load();
});

const startupOptions = [
  { label: "聊天", value: "chat" },
  { label: "上次", value: "last" },
  { label: "无", value: "none" },
];

const onStartup = computed<string>({
  get: () => settingsStore.prefs.on_startup ?? "chat",
  set: (v: string) => settingsStore.update("on_startup", v),
});

const defaultAgentId = computed<string>({
  get: () => settingsStore.prefs.default_agent_id ?? "",
  set: (v: string) => settingsStore.update("default_agent_id", v || null),
});

const defaultTemplateId = computed<string>({
  get: () => settingsStore.prefs.default_template_id ?? "",
  set: (v: string) => settingsStore.update("default_template_id", v || null),
});

function onAgentChange(e: Event): void {
  defaultAgentId.value = (e.target as HTMLInputElement).value;
}
function onTemplateChange(e: Event): void {
  defaultTemplateId.value = (e.target as HTMLInputElement).value;
}
</script>

<template>
  <div class="settings-general">
    <h2 class="section-title">通用</h2>
    <SettingRow label="默认 Agent" description="新建会话时使用的 Agent">
      <select class="select-control" :value="defaultAgentId" @change="onAgentChange">
        <option value="">自动（第一个）</option>
        <option v-for="agent in agentsStore.agents" :key="agent.id" :value="agent.id">
          {{ agent.name }}
        </option>
      </select>
    </SettingRow>
    <SettingRow label="默认模板" description="新建会话时注入的模板">
      <select class="select-control" :value="defaultTemplateId" @change="onTemplateChange">
        <option value="">无</option>
        <option v-for="tpl in templatesStore.templates" :key="tpl.id" :value="tpl.id">
          {{ tpl.name }}
        </option>
      </select>
    </SettingRow>
    <SettingRow label="启动时自动打开" description="应用启动后显示的页面">
      <SegmentedControl v-model="onStartup" :options="startupOptions" />
    </SettingRow>
    <SettingRow label="语言" description="界面显示语言">
      <select class="select-control" disabled>
        <option>简体中文</option>
      </select>
    </SettingRow>
  </div>
</template>

<style scoped>
.settings-general { max-width: 640px; }
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
