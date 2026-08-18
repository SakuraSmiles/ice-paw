<script setup lang="ts">
// ProjectContextEditor.vue — 项目背景（project.md / conventions.md）编辑区共享组件。
// 双入口复用：ProjectList 展开区 + 项目详情页设置 tab。状态完全自持
// （加载/脏检查/分文件保存/打开目录整体从 ProjectList 搬入），数据走
// project store 单条缓存——两入口编辑同一项目自然同步。挂载即 force 加载
// （防外部编辑器改后陈旧）。
import { ref, computed, watch } from "vue";
import { useProjectStore } from "../../stores/project";
import { bridge } from "../../api/bridge";

const props = defineProps<{ projectId: string }>();

const project = useProjectStore();

const loading = ref(false);
const available = ref(true);
const dir = ref<string | null>(null);
const projectMd = ref("");
const conventionsMd = ref("");
const originalMd = ref("");
const originalConv = ref("");
const showConventions = ref(false);
const error = ref("");
const saving = ref(false);

const dirty = computed(
  () => projectMd.value !== originalMd.value || conventionsMd.value !== originalConv.value,
);

function load() {
  loading.value = true;
  error.value = "";
  project
    .loadContext(props.projectId, true)
    .then((c) => {
      available.value = c.available;
      dir.value = c.dir ?? null;
      projectMd.value = c.project_md;
      conventionsMd.value = c.conventions_md;
      originalMd.value = c.project_md;
      originalConv.value = c.conventions_md;
    })
    .catch((e) => {
      error.value = e instanceof Error ? e.message : "加载项目背景失败";
    })
    .finally(() => {
      loading.value = false;
    });
}

watch(() => props.projectId, () => {
  showConventions.value = false;
  load();
}, { immediate: true });

async function save() {
  if (saving.value || !dirty.value) return;
  saving.value = true;
  error.value = "";
  try {
    if (projectMd.value !== originalMd.value) {
      await project.saveContext(props.projectId, "project.md", projectMd.value);
      originalMd.value = projectMd.value;
    }
    if (conventionsMd.value !== originalConv.value) {
      await project.saveContext(props.projectId, "conventions.md", conventionsMd.value);
      originalConv.value = conventionsMd.value;
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : "保存项目背景失败";
  } finally {
    saving.value = false;
  }
}

async function openDir() {
  try {
    await bridge.projects.openContextDir(props.projectId);
  } catch (e) {
    console.error("打开项目上下文目录失败:", e);
  }
}
</script>

<template>
  <div class="field">
    <div class="field-label">
      <span>项目背景</span>
      <span class="hint">project.md · 注入本项目全部会话，修改即时生效</span>
      <button
        v-if="dir"
        type="button"
        class="ctx-dir-btn"
        :title="dir"
        @click="openDir"
      >打开目录</button>
    </div>
    <div v-if="!available" class="ctx-guide">
      未解析到默认工作区，项目背景暂不可用——请在「设置 → 通用」确认默认工作区后重试。
    </div>
    <template v-else>
      <textarea
        v-model="projectMd"
        class="ctx-md"
        rows="10"
        :disabled="loading"
        placeholder="# 项目说明&#10;&#10;技术栈 / 架构 / 业务背景 / 术语表……项目内所有会话都会带上这份背景"
      ></textarea>
      <button type="button" class="conv-toggle" @click="showConventions = !showConventions">
        <svg class="chev" :class="{ rotated: showConventions }" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6" /></svg>
        <span>编码规范 conventions.md</span>
        <span class="hint">可选，与项目背景一同注入</span>
      </button>
      <textarea
        v-show="showConventions"
        v-model="conventionsMd"
        class="ctx-md"
        rows="6"
        :disabled="loading"
        placeholder="命名 / 格式 / 最佳实践……"
      ></textarea>
      <div class="ctx-actions">
        <button
          class="btn btn-primary btn-sm"
          :disabled="!dirty || saving || loading"
          @click="save"
        >{{ saving ? "保存中" : "保存项目背景" }}</button>
        <span v-if="!dirty && !loading && !saving" class="ctx-saved">已与文件同步</span>
      </div>
      <p class="field-hint">文件存放在 IcePaw 工作区（projects/{{ projectId }}/），不进项目源码目录</p>
    </template>
    <div v-if="error" class="form-error">{{ error }}</div>
  </div>
</template>

<style scoped>
/* 样式自持（从 ProjectList 编辑区原样搬入），不依赖父级 scoped CSS */
.field { display: flex; flex-direction: column; gap: 6px; }
.field-label {
  font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  display: flex; align-items: center; gap: 6px;
}
.hint { color: var(--ip-color-text-tertiary); font-weight: var(--ip-font-weight-regular); }

.ctx-dir-btn {
  margin-left: auto; height: 22px; padding: 0 8px;
  font-size: var(--ip-text-caption-size); font-family: inherit;
  color: var(--ip-color-text-tertiary); background: none;
  border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-full);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.ctx-dir-btn:hover { color: var(--ip-primary-600); border-color: var(--ip-primary-300); }
.ctx-guide {
  padding: 10px 12px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-md);
}
.ctx-md {
  width: 100%; padding: 10px 12px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-md);
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-caption-size); line-height: 1.7;
  color: var(--ip-color-text-primary);
  resize: vertical;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.ctx-md:focus { outline: none; border-color: var(--color-input-focus-border); background-color: var(--color-input-bg); }
.ctx-md:disabled { opacity: 0.6; }
.ctx-md::placeholder { color: var(--ip-color-text-disabled); }
.conv-toggle {
  display: flex; align-items: center; gap: 6px;
  background: none; border: none; padding: 0; cursor: pointer;
  font-family: inherit; font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.conv-toggle:hover { color: var(--ip-color-text-secondary); }
.chev { transition: transform var(--ip-duration-fast) var(--ip-ease-out); }
.chev.rotated { transform: rotate(90deg); }
.ctx-actions { display: flex; align-items: center; gap: 8px; }
.ctx-saved { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); }
.field-hint { margin: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); line-height: 1.5; }
.form-error { font-size: var(--ip-text-caption-size); color: var(--ip-danger-text); }

.btn {
  display: inline-flex; align-items: center; justify-content: center; gap: 6px;
  border-radius: var(--ip-radius-md); cursor: pointer; font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-sm { height: 30px; padding: 0 14px; font-size: var(--ip-text-body-sm-size); }
.btn-primary { background-color: var(--ip-primary-500); color: white; }
.btn-primary:hover:not(:disabled) { background-color: var(--ip-primary-600); }
.btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
