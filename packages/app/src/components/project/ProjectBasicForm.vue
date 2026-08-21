<script setup lang="ts">
// ProjectBasicForm.vue — 项目基础信息表单（名称/描述/源码目录/图标与颜色）共享组件。
// 双入口复用：ProjectList 展开区 + 项目详情页设置 tab。目录选择内聚
// （plugin-dialog），值走单对象 v-model——父级持有表单状态，本组件无状态
// （头像处理的瞬时错误提示除外——不进表单值）。
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import EntityAvatar from "../common/EntityAvatar.vue";
import { compressAvatar } from "../../utils/avatar";

interface ProjectBasicValue {
  name: string;
  description: string;
  workspacePath: string;
  /** 头像图片（base64 dataURL；空走名字渐变兜底） */
  avatar: string | null;
  /** 主题色 hex；null = 无 */
  themeColor: string | null;
}

const props = defineProps<{ modelValue: ProjectBasicValue }>();
const emit = defineEmits<{ "update:modelValue": [value: ProjectBasicValue] }>();

/** 策展主题色（10 档；与 EntityAvatar 渐变色板同族，双主题可读） */
const THEME_COLORS: ReadonlyArray<string> = [
  "#4680C2", "#3BAF7A", "#B8862A", "#B83D3D", "#7C6BC4",
  "#3D9DB3", "#C46B9A", "#6FA1D6", "#8A9B4A", "#8D8D9E",
];

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

// ---- 图标与颜色行（图片独立，主题色独立） ----
const avatarInput = ref<HTMLInputElement | null>(null);
const avatarError = ref("");

async function onAvatarFile(e: Event) {
  const input = e.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = ""; // 允许重复选同一文件
  if (!file) return;
  avatarError.value = "";
  try {
    patch({ avatar: await compressAvatar(file) });
  } catch (err) {
    avatarError.value = err instanceof Error ? err.message : "图片处理失败";
  }
}

function clearAvatar() {
  patch({ avatar: null });
  avatarError.value = "";
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

    <!-- 图标与颜色（身份出生证：图片/名字渐变两级 + 主题色点缀） -->
    <div class="field">
      <label class="field-label">图标与颜色 <span class="opt">可选</span></label>
      <div class="icon-row">
        <EntityAvatar
          :name="modelValue.name || '?'"
          :image="modelValue.avatar"
          :accent="modelValue.themeColor"
          size="lg"
        />
        <div class="icon-actions">
          <button type="button" class="icon-btn" @click="avatarInput?.click()">上传图片</button>
          <button
            v-if="modelValue.avatar"
            type="button"
            class="icon-btn"
            @click="clearAvatar"
          >清除</button>
          <span v-if="avatarError" class="icon-err">{{ avatarError }}</span>
        </div>
        <input ref="avatarInput" type="file" accept="image/*" class="avatar-file" @change="onAvatarFile" />
      </div>
      <div class="swatch-row">
        <span class="swatch-label">主题色</span>
        <button
          v-for="c in THEME_COLORS"
          :key="c"
          type="button"
          class="swatch"
          :class="{ active: modelValue.themeColor === c }"
          :style="{ background: c }"
          :title="c"
          @click="patch({ themeColor: c })"
        />
        <button
          type="button"
          class="swatch swatch-none"
          :class="{ active: !modelValue.themeColor }"
          title="不使用主题色"
          @click="patch({ themeColor: null })"
        >无</button>
      </div>
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
.opt { color: var(--ip-color-text-tertiary); font-weight: var(--ip-font-weight-regular); }

.input {
  height: 34px; padding: 0 10px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-primary);
  font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.input:focus { outline: none; border-color: var(--ip-color-border-focus); background-color: var(--ip-color-bg-input); }

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

/* 图标与颜色行 */
.icon-row { display: flex; align-items: center; gap: var(--ip-spacing-2_5); }
.icon-actions { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.icon-btn {
  height: 22px; padding: 0 10px;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: none; border-radius: var(--ip-radius-full);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.icon-btn:hover { color: var(--ip-color-text-primary); background-color: var(--ip-color-bg-secondary); }
.icon-err { font-size: var(--ip-text-micro-size); color: var(--ip-danger-text); }
.avatar-file { display: none; }

/* 主题色 swatch 行 */
.swatch-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.swatch-label { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.swatch {
  width: 18px; height: 18px; flex-shrink: 0;
  border-radius: var(--ip-radius-full);
  border: 2px solid transparent;
  cursor: pointer;
  padding: 0;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.swatch:hover { transform: scale(1.15); }
.swatch.active { border-color: var(--ip-color-bg-primary); outline: 1.5px solid var(--ip-color-text-primary); }
.swatch-none {
  width: auto; height: 18px; padding: 0 8px;
  background: transparent;
  border: 1px dashed var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary);
}
.swatch-none:hover { color: var(--ip-color-text-primary); transform: none; }
</style>
