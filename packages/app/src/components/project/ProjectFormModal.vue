<script setup lang="ts">
/**
 * ProjectFormModal — 项目创建/编辑弹窗
 *
 * 复用两态：
 *   - mode='create' → 调用 createWithAgents(input)
 *   - mode='edit'   → 调用 updateFull(id, patch, members) 原子提交
 *
 * 设计要点：
 *   - 由父组件传入 `agents`（避免组件内调 store 形成循环依赖）
 *   - workspace_path 提供「浏览…」按钮调用 Tauri dialog.open({ directory: true })
 *   - 成员编辑器：左侧 select + 添加按钮；右侧已添加 chips 含 role select + 删除
 */

import { computed, ref, watch } from "vue";
import { Modal } from "@ice-paw/ui";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FolderSearch, Plus, X } from "lucide-vue-next";
import { PROJECT_ICON_OPTIONS, resolveProjectIcon } from "../../utils/projectIconMap";
import type { Agent, Project, ProjectMemberInput, NewProject, ProjectPatch } from "../../types";

type Mode = "create" | "edit";

interface Props {
  modelValue: boolean;
  mode: Mode;
  /** 编辑模式必传，创建模式忽略 */
  initial?: Project | null;
  /** 由父组件传入的全部 Agent 列表 */
  agents: Agent[];
}

interface Emits {
  (e: "update:modelValue", value: boolean): void;
  (e: "submit-create", payload: NewProject): void;
  (e: "submit-edit", payload: { patch: ProjectPatch; members: ProjectMemberInput[] }): void;
  (e: "close"): void;
}

const props = withDefaults(defineProps<Props>(), {
  initial: null,
});
const emit = defineEmits<Emits>();

// ===== 表单状态（受控） =====
const name = ref<string>("");
const description = ref<string>("");
const icon = ref<string>("folder");
const workspacePath = ref<string>("");
const members = ref<ProjectMemberInput[]>([]);
const errors = ref<Record<string, string>>({});
const submitting = ref<boolean>(false);

// 预定义图标（lucide 图标名，与 ProjectWelcome 等场景对齐）
const ICON_OPTIONS = PROJECT_ICON_OPTIONS;

// ===== 标题 =====
const title = computed<string>(() =>
  props.mode === "create" ? "新建项目" : "编辑项目",
);

// ===== 已添加成员 ID 集合（用于排除下拉项） =====
const memberIdSet = computed<Set<string>>(() => {
  return new Set(members.value.map((m) => m.agent_id));
});

// ===== 可选 Agent（已排除已添加） =====
const availableAgents = computed<Agent[]>(() => {
  return props.agents.filter((a) => !memberIdSet.value.has(a.id));
});

// ===== 名称校验 =====
function nameError(): string {
  if (!name.value.trim()) return "项目名称不能为空";
  if (name.value.trim().length > 60) return "项目名称过长（最多 60 字符）";
  return "";
}

function workspaceError(): string {
  if (!workspacePath.value.trim()) return ""; // 允许空
  // 极简客户端校验：仅校验非过长；后端命令会再校验存在性
  if (workspacePath.value.length > 4096) return "路径过长";
  return "";
}

// ===== 初始化 / 同步 initial =====
watch(
  () => [props.modelValue, props.mode, props.initial?.id],
  () => {
    if (!props.modelValue) return; // 关闭时不重置，避免父组件销毁态被覆盖
    if (props.mode === "edit" && props.initial) {
      name.value = props.initial.name;
      description.value = props.initial.description;
      icon.value = props.initial.icon || "folder";
      workspacePath.value = props.initial.workspace_path ?? "";
      members.value = props.initial.agents.map((m) => ({
        agent_id: m.agent_id,
        role: m.role,
      }));
    } else {
      name.value = "";
      description.value = "";
      icon.value = "folder";
      workspacePath.value = "";
      members.value = [];
    }
    errors.value = {};
    submitting.value = false;
  },
  { immediate: true },
);

// ===== 浏览工作区目录 =====
async function pickWorkspace(): Promise<void> {
  try {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: "选择项目工作区目录",
    });
    if (typeof picked === "string") {
      workspacePath.value = picked;
    }
  } catch (e) {
    // 静默失败，前端不强制校验
    console.warn("[ProjectFormModal] openDialog failed:", e);
  }
}

// ===== 添加成员 =====
function addMember(agentId: string): void {
  if (!agentId) return;
  if (memberIdSet.value.has(agentId)) return;
  members.value.push({ agent_id: agentId, role: "member" });
}

function removeMember(agentId: string): void {
  members.value = members.value.filter((m) => m.agent_id !== agentId);
}

function setMemberRole(agentId: string, role: "lead" | "member"): void {
  const m = members.value.find((m) => m.agent_id === agentId);
  if (m) m.role = role;
}

// ===== 提交 =====
async function handleSubmit(): Promise<void> {
  const ne = nameError();
  const we = workspaceError();
  if (ne) {
    errors.value = { name: ne };
    return;
  }
  if (we) {
    errors.value = { workspace_path: we };
    return;
  }

  submitting.value = true;
  try {
    if (props.mode === "create") {
      const payload: NewProject = {
        name: name.value.trim(),
        description: description.value.trim() || undefined,
        icon: icon.value,
        workspace_path: workspacePath.value.trim() || null,
        agents: members.value.map((m) => ({ agent_id: m.agent_id, role: m.role })),
      };
      emit("submit-create", payload);
    } else {
      // 编辑模式：基础字段作为 patch 提交；成员列表一并原子提交
      const patch: ProjectPatch = {
        name: name.value.trim(),
        description: description.value.trim() || null,
        icon: icon.value,
        workspace_path: workspacePath.value.trim() || null,
      };
      emit("submit-edit", {
        patch,
        members: members.value.map((m) => ({ agent_id: m.agent_id, role: m.role })),
      });
    }
    emit("update:modelValue", false);
  } finally {
    submitting.value = false;
  }
}

function onClose(): void {
  emit("update:modelValue", false);
  emit("close");
}

// ===== 工具：根据 agent_id 取 Agent 名（展示用） =====
function agentName(agentId: string): string {
  return props.agents.find((a) => a.id === agentId)?.name ?? agentId;
}
</script>

<template>
  <Modal
    :model-value="modelValue"
    :title="title"
    size="md"
    :close-on-overlay="!submitting"
    :close-on-esc="!submitting"
    @update:model-value="onClose"
  >
    <!-- 表单主体 -->
    <form class="pf-form" @submit.prevent="handleSubmit">
      <!-- 名称 -->
      <div class="pf-field">
        <label class="pf-label" for="pf-name">项目名称 <span class="pf-required">*</span></label>
        <input
          id="pf-name"
          v-model="name"
          class="pf-input"
          type="text"
          maxlength="60"
          placeholder="例如：ice-paw 0.2 重构"
          :disabled="submitting"
        />
        <span v-if="errors.name" class="pf-error">{{ errors.name }}</span>
      </div>

      <!-- 描述 -->
      <div class="pf-field">
        <label class="pf-label" for="pf-desc">描述</label>
        <textarea
          id="pf-desc"
          v-model="description"
          class="pf-input pf-textarea"
          rows="2"
          maxlength="500"
          placeholder="项目目的 / 关键约束（可选）"
          :disabled="submitting"
        />
      </div>

      <!-- 图标 -->
      <div class="pf-field">
        <label class="pf-label">图标</label>
        <div class="pf-icon-grid">
          <button
            v-for="opt in ICON_OPTIONS"
            :key="opt"
            type="button"
            :class="['pf-icon-btn', { 'pf-icon-btn-active': icon === opt }]"
            :disabled="submitting"
            @click="icon = opt"
          >
            <component :is="resolveProjectIcon(opt)" :size="18" />
          </button>
        </div>
      </div>

      <!-- 工作区路径 -->
      <div class="pf-field">
        <label class="pf-label" for="pf-ws">项目空间（本地路径）</label>
        <div class="pf-ws-row">
          <input
            id="pf-ws"
            v-model="workspacePath"
            class="pf-input pf-ws-input"
            type="text"
            placeholder="例如：/Users/dabai/Projects/ice-paw"
            :disabled="submitting"
          />
          <button
            type="button"
            class="pf-btn pf-btn-secondary"
            :disabled="submitting"
            @click="pickWorkspace"
          >
            <FolderSearch :size="14" aria-hidden="true" />
            <span>浏览…</span>
          </button>
        </div>
        <span v-if="errors.workspace_path" class="pf-error">{{ errors.workspace_path }}</span>
        <span class="pf-hint">Agent 在工具调用时会以该路径作为根目录</span>
      </div>

      <!-- Agent 成员 -->
      <div class="pf-field">
        <label class="pf-label">Agent 成员</label>

        <!-- 已添加 -->
        <div v-if="members.length > 0" class="pf-member-list">
          <div
            v-for="m in members"
            :key="m.agent_id"
            class="pf-member-item"
          >
            <span class="pf-member-name">{{ agentName(m.agent_id) }}</span>
            <select
              class="pf-member-role"
              :value="m.role"
              :disabled="submitting"
              @change="(e) => setMemberRole(m.agent_id, (e.target as HTMLSelectElement).value as 'lead' | 'member')"
            >
              <option value="lead">lead</option>
              <option value="member">member</option>
            </select>
            <button
              type="button"
              class="pf-member-remove"
              :disabled="submitting"
              title="移除"
              @click="removeMember(m.agent_id)"
            >
              <X :size="14" aria-hidden="true" />
            </button>
          </div>
        </div>
        <div v-else class="pf-empty">暂无成员，可在下方下拉中添加</div>

        <!-- 添加 -->
        <div v-if="availableAgents.length > 0" class="pf-add-row">
          <Plus :size="14" class="pf-add-icon" aria-hidden="true" />
          <select
            class="pf-agent-select"
            :disabled="submitting"
            @change="(e) => { const v = (e.target as HTMLSelectElement).value; if (v) { addMember(v); (e.target as HTMLSelectElement).value = ''; } }"
          >
            <option value="">添加 Agent…</option>
            <option v-for="a in availableAgents" :key="a.id" :value="a.id">
              {{ a.name }}
            </option>
          </select>
        </div>
        <div v-else-if="props.agents.length === 0" class="pf-hint">
          请先在 Agent 管理页创建 Agent
        </div>
      </div>
    </form>

    <!-- Footer -->
    <template #footer>
      <button
        type="button"
        class="pf-btn pf-btn-ghost"
        :disabled="submitting"
        @click="onClose"
      >
        取消
      </button>
      <button
        type="button"
        class="pf-btn pf-btn-primary"
        :disabled="submitting || !!nameError()"
        @click="handleSubmit"
      >
        {{ submitting ? "保存中…" : "保存" }}
      </button>
    </template>
  </Modal>
</template>

<style scoped>
.pf-form {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
}

.pf-field {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.pf-label {
  font-size: var(--ip-text-caption-size, 12px);
  font-weight: var(--ip-font-weight-semibold, 600);
  color: var(--ip-color-text-secondary);
}

.pf-required {
  color: var(--ip-danger-text, #dc2626);
}

.pf-input {
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm, 4px);
  outline: none;
  transition: border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
              box-shadow var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.pf-input:focus {
  border-color: var(--ip-color-border-focus, #3b82f6);
  box-shadow: var(--ip-shadow-focus, 0 0 0 3px rgba(59, 130, 246, 0.2));
}

.pf-input:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.pf-textarea {
  resize: vertical;
  min-height: 48px;
}

.pf-icon-grid {
  display: grid;
  grid-template-columns: repeat(8, 1fr);
  gap: var(--ip-spacing-1);
}

.pf-icon-btn {
  appearance: none;
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
  border-radius: var(--ip-radius-sm, 4px);
  padding: var(--ip-spacing-2) 0;
  font-size: 18px;
  cursor: pointer;
  transition: border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
              background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.pf-icon-btn:hover {
  background: var(--ip-color-bg-tertiary);
}

.pf-icon-btn-active {
  border-color: var(--ip-primary-500, #3b82f6);
  background: var(--ip-primary-50, #eff6ff);
}

.pf-ws-row {
  display: flex;
  gap: var(--ip-spacing-2);
  align-items: stretch;
}

.pf-ws-input {
  flex: 1;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 12px;
}

.pf-error {
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-danger-text, #dc2626);
}

.pf-hint {
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-tertiary);
}

.pf-empty {
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-tertiary);
  background: var(--ip-color-bg-secondary);
  border: 1px dashed var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm, 4px);
  text-align: center;
}

.pf-member-list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
  margin-bottom: var(--ip-spacing-2);
}

.pf-member-item {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm, 4px);
}

.pf-member-name {
  flex: 1;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-primary);
}

.pf-member-role {
  padding: var(--ip-spacing-1) var(--ip-spacing-2);
  font-family: inherit;
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm, 4px);
  cursor: pointer;
}

.pf-member-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: var(--ip-radius-sm, 4px);
  background: transparent;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
              color var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.pf-member-remove:hover {
  background: var(--ip-danger-bg, #fef2f2);
  color: var(--ip-danger-text, #dc2626);
}

.pf-add-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}

.pf-add-icon {
  color: var(--ip-color-text-tertiary);
}

.pf-agent-select {
  flex: 1;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm, 4px);
  cursor: pointer;
  outline: none;
}

.pf-btn {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  padding: var(--ip-spacing-1) var(--ip-spacing-4);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size, 13px);
  font-weight: var(--ip-font-weight-medium, 500);
  border-radius: var(--ip-radius-sm, 4px);
  border: 1px solid transparent;
  cursor: pointer;
  transition: background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
              border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.pf-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.pf-btn-primary {
  background: var(--ip-primary-500, #3b82f6);
  color: white;
  border-color: var(--ip-primary-500, #3b82f6);
}

.pf-btn-primary:hover:not(:disabled) {
  background: var(--ip-primary-600, #2563eb);
  border-color: var(--ip-primary-600, #2563eb);
}

.pf-btn-secondary {
  background: var(--ip-color-bg-primary);
  color: var(--ip-color-text-secondary);
  border-color: var(--ip-color-border-default);
}

.pf-btn-secondary:hover:not(:disabled) {
  background: var(--ip-color-bg-tertiary);
}

.pf-btn-ghost {
  background: transparent;
  color: var(--ip-color-text-secondary);
  border-color: var(--ip-color-border-default);
}

.pf-btn-ghost:hover:not(:disabled) {
  background: var(--ip-color-bg-tertiary);
}
</style>
