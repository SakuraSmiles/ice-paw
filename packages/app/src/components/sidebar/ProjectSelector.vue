<script setup lang="ts">
// 项目选择器（Phase 2）
//
// 职责：
//   - 顶部横条展示当前项目名 + 图标 + 下拉箭头
//   - 点击展开下拉列表（absolute 定位）
//   - 下拉项：第一项固定为「默认项目」，后续为用户创建的项目
//   - 底部：「+ 新建项目」内联创建、「管理项目 →」跳转路由
//   - 点击切换项目 → projectsStore.setCurrent(id) → 触发会话列表重新加载
//
// 数据源：projectsStore
// 设计参考：AgentSelector.vue（保持一致的视觉风格与交互模式）

import { computed, ref, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { ChevronDown, FolderClosed, Plus, ArrowRight, Clipboard } from "lucide-vue-next";
import PawBrandMark from "../common/PawBrandMark.vue";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../../stores/projects";
import { useToast } from "../../composables/useToast";
import type { Project } from "../../types";

const projectsStore = useProjectsStore();
const router = useRouter();
const toast = useToast();

/** 下拉是否展开 */
const expanded = ref<boolean>(false);

/** 是否显示内联新建项目输入框 */
const showCreate = ref<boolean>(false);

/** 新项目名称（内联创建输入） */
const newProjectName = ref<string>("");

/** 新项目描述 */
const newProjectDesc = ref<string>("");

/** 是否正在创建项目 */
const creating = ref<boolean>(false);

/** 当前项目名 */
const currentName = computed<string>(() => {
  return projectsStore.current?.name ?? "未选择";
});

/** 排序后的项目列表（含默认项目在首位） */
const projectList = computed<Project[]>(() => {
  return projectsStore.sortedProjects;
});

/** 切换展开 */
function toggle(): void {
  expanded.value = !expanded.value;
}

/** 选中项目 */
function pick(id: string): void {
  projectsStore.setCurrent(id);
  expanded.value = false;
}

/** 跳转到项目管理页 */
function goToManager(): void {
  expanded.value = false;
  const projectId =
    projectsStore.currentId === DEFAULT_PROJECT_ID
      ? "default"
      : projectsStore.currentId;
  void router.push({ name: "ProjectSettings", params: { projectId } });
}

/** 显示内联创建 */
function startCreate(): void {
  showCreate.value = true;
  newProjectName.value = "";
  newProjectDesc.value = "";
}

/** 取消内联创建 */
function cancelCreate(): void {
  showCreate.value = false;
  newProjectName.value = "";
  newProjectDesc.value = "";
}

/** 确认创建项目 */
async function confirmCreate(): Promise<void> {
  const name = newProjectName.value.trim();
  if (!name) {
    toast.warning("项目名称不能为空");
    return;
  }
  creating.value = true;
  try {
    const created = await projectsStore.create({
      name,
      description: newProjectDesc.value.trim() || undefined,
    });
    projectsStore.setCurrent(created.id);
    showCreate.value = false;
    newProjectName.value = "";
    newProjectDesc.value = "";
    expanded.value = false;
  } catch {
    toast.error("创建项目失败");
  } finally {
    creating.value = false;
  }
}

/** 点击外部关闭 */
function onDocumentClick(e: MouseEvent): void {
  if (!expanded.value && !showCreate.value) return;
  const target = e.target as HTMLElement | null;
  if (target && target.closest(".project-selector")) return;
  expanded.value = false;
  if (showCreate.value && !creating.value) {
    cancelCreate();
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
});

onUnmounted(() => {
  document.removeEventListener("click", onDocumentClick);
});
</script>

<template>
  <div class="project-selector">
    <button
      :class="['selector-trigger', { 'selector-trigger-open': expanded }]"
      type="button"
      :aria-expanded="expanded"
      aria-haspopup="listbox"
      @click.stop="toggle"
    >
      <span class="selector-label">项目</span>
      <PawBrandMark
        :size="24"
        :animated="true"
        class="selector-icon"
      />
      <span class="selector-name">{{ currentName }}</span>
      <ChevronDown
        :size="14"
        :class="['selector-arrow', { 'selector-arrow-open': expanded }]"
        aria-hidden="true"
      />
    </button>

    <Transition name="dropdown">
      <div v-if="expanded" class="selector-dropdown" role="listbox" @click.stop>
        <!-- 默认项目（固定第一项） -->
        <button
          :class="[
            'dropdown-item',
            projectsStore.currentId === DEFAULT_PROJECT_ID ? 'dropdown-item-active' : '',
          ]"
          type="button"
          role="option"
          :aria-selected="projectsStore.currentId === DEFAULT_PROJECT_ID"
          @click="pick(DEFAULT_PROJECT_ID)"
        >
          <div class="item-row">
            <Clipboard :size="16" class="item-icon-lucide" aria-hidden="true" />
            <div class="item-text">
              <span class="item-name">默认项目</span>
              <span class="item-desc">未分配项目的会话</span>
            </div>
          </div>
        </button>

        <!-- 用户创建的项目 -->
        <button
          v-for="proj in projectList"
          :key="proj.id"
          :class="[
            'dropdown-item',
            projectsStore.currentId === proj.id ? 'dropdown-item-active' : '',
          ]"
          type="button"
          role="option"
          :aria-selected="projectsStore.currentId === proj.id"
          @click="pick(proj.id)"
        >
          <div class="item-row">
            <FolderClosed :size="16" class="item-icon-lucide" aria-hidden="true" />
            <div class="item-text">
              <span class="item-name">{{ proj.name }}</span>
              <span v-if="proj.description" class="item-desc">{{ proj.description }}</span>
              <span v-if="proj.agents.length > 0" class="item-meta">{{ proj.agents.length }} 个 Agent</span>
            </div>
          </div>
        </button>

        <div class="dropdown-divider" />

        <!-- 新建项目 -->
        <div v-if="showCreate" class="inline-create" @click.stop>
          <input
            v-model="newProjectName"
            class="create-input"
            type="text"
            placeholder="项目名称"
            :disabled="creating"
            @keyup.enter="confirmCreate"
            @keyup.escape="cancelCreate"
          />
          <input
            v-model="newProjectDesc"
            class="create-input create-input-desc"
            type="text"
            placeholder="描述（可选）"
            :disabled="creating"
            @keyup.enter="confirmCreate"
            @keyup.escape="cancelCreate"
          />
          <div class="create-actions">
            <button
              class="create-btn create-btn-cancel"
              type="button"
              :disabled="creating"
              @click="cancelCreate"
            >
              取消
            </button>
            <button
              class="create-btn create-btn-confirm"
              type="button"
              :disabled="creating"
              @click="confirmCreate"
            >
              {{ creating ? "创建中…" : "创建" }}
            </button>
          </div>
        </div>

        <button v-if="!showCreate" class="dropdown-item dropdown-action" type="button" @click="startCreate">
          <Plus :size="14" aria-hidden="true" />
          <span>新建项目</span>
        </button>

        <button class="dropdown-item dropdown-manage" type="button" @click="goToManager">
          <span>管理项目</span>
          <ArrowRight :size="14" aria-hidden="true" />
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.project-selector {
  position: relative;
  padding: var(--ip-spacing-3);
  border-bottom: 1px solid var(--ip-color-border-default);
}

.selector-trigger {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  transition: var(--ip-transition-colors);
}

.selector-trigger:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}

.selector-trigger:focus-visible {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.selector-trigger-open {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}

.selector-label {
  font-size: 10px;
  font-weight: var(--ip-font-weight-semibold);
  text-transform: uppercase;
  color: var(--ip-color-text-tertiary);
  letter-spacing: 0.05em;
  flex-shrink: 0;
}

.selector-icon {
  flex-shrink: 0;
  color: var(--ip-color-text-secondary);
}

.selector-name {
  flex: 1;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.selector-arrow {
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out), color var(--ip-duration-immediate) var(--ip-ease-out);
}

.selector-arrow-open {
  transform: rotate(180deg);
  color: var(--ip-color-text-secondary);
}

.selector-dropdown {
  position: absolute;
  top: calc(100% - 1px);
  left: var(--ip-spacing-3);
  right: var(--ip-spacing-3);
  z-index: var(--ip-z-dropdown);
  max-height: 360px;
  overflow-y: auto;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  padding: var(--ip-spacing-1);
  display: flex;
  flex-direction: column;
}

.dropdown-item {
  appearance: none;
  border: none;
  background: transparent;
  text-align: left;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  transition: var(--ip-transition-colors);
}

.dropdown-item:hover {
  background: var(--ip-color-bg-tertiary);
}

.dropdown-item:focus-visible {
  outline: none;
  background: var(--ip-color-bg-tertiary);
  box-shadow: var(--ip-shadow-focus);
}

/* 选中态 */
.dropdown-item-active {
  background: var(--ip-primary-50);
  color: var(--ip-primary-700);
}

.dropdown-item-active:hover {
  background: var(--ip-primary-100);
}

[data-theme="dark"] .dropdown-item-active {
  background: var(--ip-primary-900);
  color: var(--ip-primary-100);
}

[data-theme="dark"] .dropdown-item-active:hover {
  background: var(--ip-primary-800);
}

/* 下拉项内部布局 */
.item-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}

.item-icon {
  font-size: 16px;
  flex-shrink: 0;
  line-height: 1;
}

.item-icon-lucide {
  color: var(--ip-color-text-secondary);
  flex-shrink: 0;
}

.dropdown-item-active .item-icon-lucide {
  color: var(--ip-primary-700);
}

[data-theme="dark"] .dropdown-item-active .item-icon-lucide {
  color: var(--ip-primary-100);
}

.item-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1;
}

.item-name {
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-desc {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-meta {
  font-size: 10px;
  color: var(--ip-color-text-tertiary);
  opacity: 0.8;
}

.dropdown-item-active .item-desc {
  color: var(--ip-primary-600);
}

[data-theme="dark"] .dropdown-item-active .item-desc {
  color: var(--ip-primary-300);
}

.dropdown-divider {
  height: 1px;
  margin: var(--ip-spacing-1) var(--ip-spacing-2);
  background: var(--ip-color-border-default);
}

/* 内联创建 */
.inline-create {
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.create-input {
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  outline: none;
  transition: var(--ip-transition-colors);
}

.create-input:focus {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.create-input::placeholder {
  color: var(--ip-color-text-tertiary);
}

.create-input-desc {
  font-size: var(--ip-text-caption-size);
}

.create-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--ip-spacing-2);
}

.create-btn {
  padding: var(--ip-spacing-1) var(--ip-spacing-3);
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-sm);
  border: 1px solid var(--ip-color-border-default);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.create-btn-cancel {
  background: var(--ip-color-bg-primary);
  color: var(--ip-color-text-secondary);
}

.create-btn-cancel:hover {
  background: var(--ip-color-bg-tertiary);
}

.create-btn-confirm {
  background: var(--ip-primary-500);
  color: white;
  border-color: var(--ip-primary-500);
}

.create-btn-confirm:hover {
  background: var(--ip-primary-600);
  border-color: var(--ip-primary-600);
}

.create-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* 动作按钮 */
.dropdown-action {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-link);
}

.dropdown-action:hover {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-link);
}

.dropdown-manage {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-link);
}

.dropdown-manage:hover {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-link);
}

/* 下拉动画 */
.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
.dropdown-enter-active,
.dropdown-leave-active {
  transition:
    opacity var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out);
}
</style>
