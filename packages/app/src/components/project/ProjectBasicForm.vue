<script setup lang="ts">
// ProjectBasicForm.vue — 项目基础信息表单（名称/描述/源码目录）共享组件。
// 双入口复用：ProjectList 展开区 + 项目详情页设置 tab。目录选择内聚
// （plugin-dialog），值走单对象 v-model——父级持有表单状态，本组件无状态。
import { open } from "@tauri-apps/plugin-dialog";

interface ProjectBasicValue {
  name: string;
  description: string;
  workspacePath: string;
}

const props = defineProps<{ modelValue: ProjectBasicValue }>();
const emit = defineEmits<{ "update:modelValue": [value: ProjectBasicValue] }>();

/** 局部字段更新（单对象 v-model 的对象整体替换，props 不就地改） */
function patch(part: Partial<ProjectBasicValue>) {
  emit("update:modelValue", { ...props.modelValue, ...part });
}

async function pickWorkspace() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择项目源码目录",
    defaultPath: props.modelValue.workspacePath || undefined,
  });
  if (selected) patch({ workspacePath: selected });
}
</script>

<template>
  <div class="basic-form">
    <div class="field">
      <label class="field-label">名称 <span class="req">*</span></label>
      <input
        :value="modelValue.name"
        type="text"
        class="input"
        placeholder="项目名称"
        @input="patch({ name: ($event.target as HTMLInputElement).value })"
      />
    </div>

    <div class="field">
      <label class="field-label">描述</label>
      <input
        :value="modelValue.description"
        type="text"
        class="input"
        placeholder="一句话说明项目用途（可选）"
        @input="patch({ description: ($event.target as HTMLInputElement).value })"
      />
    </div>

    <div class="field">
      <label class="field-label">源码目录</label>
      <div class="workspace-group">
        <input
          :value="modelValue.workspacePath"
          type="text"
          class="input workspace-input"
          placeholder="选择项目源码根目录（可选）"
          readonly
          @click="pickWorkspace"
        />
        <button type="button" class="ws-btn" title="选择目录" @click="pickWorkspace">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
        </button>
      </div>
      <p class="field-hint">绑定后，项目内会话的文件/代码工具切换到此目录；留空则回退 agent 工作区</p>
    </div>
  </div>
</template>

<style scoped>
/* 样式自持（从 ProjectList 编辑区原样搬入），不依赖父级 scoped CSS */
.basic-form { display: flex; flex-direction: column; gap: 14px; }

.field { display: flex; flex-direction: column; gap: 6px; }
.field-label {
  font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  display: flex; align-items: center; gap: 6px;
}
.req { color: var(--ip-danger-text); }

.input {
  height: 34px; padding: 0 10px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-primary);
  font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.input:focus { outline: none; border-color: var(--color-input-focus-border); background-color: var(--color-input-bg); }

.workspace-group { display: flex; gap: 6px; }
.workspace-input { flex: 1; cursor: pointer; font-family: var(--ip-font-mono); }
.ws-btn {
  display: flex; align-items: center; justify-content: center;
  width: 34px; height: 34px; flex-shrink: 0;
  border-radius: var(--ip-radius-md); cursor: pointer;
  background-color: var(--ip-color-bg-tertiary); border: 1px solid transparent;
  color: var(--ip-color-text-secondary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.ws-btn:hover { border-color: var(--ip-primary-300); color: var(--ip-primary-600); }

.field-hint { margin: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); line-height: 1.5; }
</style>
