<script setup lang="ts">
// GeneralSettings.vue — 通用设置
import { ref, onMounted } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { bridge } from "../../api/bridge";
import type { UserPreferences } from "../../types";

const prefs = ref<UserPreferences>({});
const loading = ref(true);
const saving = ref(false);
const saved = ref(false);

async function load() {
  loading.value = true;
  try {
    const raw = await bridge.preferences.get();
    // 统一为 / 分隔符（后端 Windows 返回 \）
    if (raw.default_workspace_path) {
      raw.default_workspace_path = raw.default_workspace_path.replace(/\\/g, "/");
    }
    prefs.value = raw;
  } catch (e) {
    console.error("加载设置失败:", e);
  } finally {
    loading.value = false;
  }
}

async function pickDirectory() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择默认工作空间目录",
    defaultPath: prefs.value.default_workspace_path || undefined,
  });
  if (selected) {
    prefs.value.default_workspace_path = selected;
  }
}

async function saveWorkspacePath() {
  saved.value = false;
  saving.value = true;
  try {
    await bridge.preferences.set(
      "default_workspace_path",
      prefs.value.default_workspace_path ?? "",
    );
    saved.value = true;
    setTimeout(() => { saved.value = false; }, 2000);
  } catch (e) {
    console.error("保存失败:", e);
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">通用</h2>
    </div>

    <div v-if="loading" class="loading-state">加载中...</div>
    <div v-else class="settings-items">
      <div class="setting-item">
        <div class="setting-label-row">
          <div class="setting-label">默认工作空间</div>
          <div class="setting-desc">
            新 Agent 未指定工作区时，自动在此目录下创建 <code>{uuid前缀}-{名称}</code> 子文件夹
          </div>
        </div>
        <div class="path-picker">
          <input
            v-model="prefs.default_workspace_path"
            type="text"
            class="form-input path-input"
            placeholder="选择或输入默认工作空间路径"
            readonly
            @click="pickDirectory"
          />
          <button class="btn-browse" @click="pickDirectory" title="选择目录">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
          </button>
        </div>
        <div class="setting-actions">
          <button class="btn-primary" :disabled="saving" @click="saveWorkspacePath">
            {{ saving ? "保存中..." : "保存" }}
          </button>
          <span v-if="saved" class="save-tip">已保存</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-content-inner { flex: 1; display: flex; flex-direction: column; padding: 24px; min-height: 0; }

.content-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 0 0 20px; flex-shrink: 0;
  height: 56px;
}
.content-title { font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); margin: 0; }

.loading-state { flex: 1; display: flex; align-items: center; justify-content: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }

.settings-items { flex: 1; }

.setting-item {
  display: flex; flex-direction: column; gap: 12px;
  padding: 20px 24px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
}

.setting-label-row { display: flex; flex-direction: column; gap: 4px; }
.setting-label { font-size: var(--ip-text-body-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.setting-desc { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); line-height: 1.5; }
.setting-desc code { font-family: var(--ip-font-mono); background: var(--ip-color-bg-tertiary); padding: 0 4px; border-radius: var(--ip-radius-sm); }

.path-picker { display: flex; gap: 8px; }
.path-input { flex: 1; height: 36px; padding: 0 12px; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-primary); background-color: var(--ip-color-bg-tertiary); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); outline: none; cursor: pointer; transition: all var(--ip-duration-fast) var(--ip-ease-out); }
.path-input:focus { border-color: var(--color-input-focus-border); background-color: var(--ip-color-bg-secondary); box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12); }
.path-input::placeholder { color: var(--ip-color-text-placeholder); }

.btn-browse {
  display: flex; align-items: center; justify-content: center;
  width: 36px; height: 36px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  cursor: pointer; flex-shrink: 0;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-browse:hover { background-color: var(--ip-color-bg-secondary); border-color: var(--color-input-focus-border); color: var(--ip-primary-600); }

.setting-actions { display: flex; align-items: center; gap: 12px; }

.btn-primary {
  display: flex; align-items: center; justify-content: center; gap: 6px;
  padding: 8px 16px; height: 36px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  color: white; background-color: var(--ip-primary-600); border: none;
  border-radius: var(--ip-radius-md); cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary:hover { background-color: var(--ip-primary-700); }
.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }

.save-tip { font-size: var(--ip-text-body-sm-size); color: var(--ip-success-text); }
</style>
