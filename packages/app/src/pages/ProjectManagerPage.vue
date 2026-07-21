<script setup lang="ts">
// 项目管理页（Phase 2）
//
// 职责：
//   - 列出所有项目（含默认项目）
//   - 项目 CRUD：创建、编辑（基本信息 + 成员）、删除
//   - 排序
//   - 删除项目前的二次确认（保留旧行为）
//
// 本页不再内联成员管理；统一由 ProjectFormModal 处理创建/编辑两态。

import { onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../stores/projects";
import { useConversationsStore } from "../stores/conversations";
import { useAgentsStore } from "../stores/agents";
import { useToast } from "../composables/useToast";
import { Plus, Trash2, ArrowLeft, Clipboard, FolderOpen } from "lucide-vue-next";
import { resolveProjectIcon } from "../utils/projectIconMap";
import ProjectFormModal from "../components/project/ProjectFormModal.vue";
import type { Project, NewProject, ProjectMemberInput, ProjectPatch } from "../types";

const projectsStore = useProjectsStore();
const conversationsStore = useConversationsStore();
const agentsStore = useAgentsStore();
const toast = useToast();
const route = useRoute();
const router = useRouter();

/** Modal 状态 */
const showModal = ref<boolean>(false);
const modalMode = ref<"create" | "edit">("create");
const editing = ref<Project | null>(null);

/** 当前选中的项目 ID（保留旧行为：用于详情区视觉聚焦） */
const selectedProjectId = ref<string>("");

onMounted(async () => {
  await projectsStore.loadAll();
  await agentsStore.ensureLoaded();
  const param = route.params.projectId as string | undefined;
  if (param && param !== "default") {
    selectedProjectId.value = param;
  }
});

function openCreate(): void {
  modalMode.value = "create";
  editing.value = null;
  showModal.value = true;
}

function openEdit(p: Project): void {
  modalMode.value = "edit";
  editing.value = p;
  showModal.value = true;
}

function selectProject(id: string): void {
  selectedProjectId.value = id;
  const routeParam = id === DEFAULT_PROJECT_ID ? "default" : id;
  void router.replace({ name: "ProjectSettings", params: { projectId: routeParam } });
}

async function handleCreate(payload: NewProject): Promise<void> {
  try {
    await projectsStore.create(payload);
    toast.success(`项目「${payload.name}」已创建`);
  } catch (e) {
    toast.error(`创建项目失败：${(e as Error).message ?? "未知错误"}`);
  }
}

async function handleEdit(payload: {
  patch: ProjectPatch;
  members: ProjectMemberInput[];
}): Promise<void> {
  if (!editing.value) return;
  try {
    // 原子提交：字段 + 成员在同一后端事务内
    await projectsStore.updateFull(
      editing.value.id,
      payload.patch,
      payload.members,
    );
    toast.success(`项目「${editing.value.name}」已更新`);
  } catch (e) {
    toast.error(`更新项目失败：${(e as Error).message ?? "未知错误"}`);
  }
}

async function handleDelete(project: Project): Promise<void> {
  if (project.id === DEFAULT_PROJECT_ID) {
    toast.warning("默认项目无法删除");
    return;
  }
  if (!confirm(`确认删除项目「${project.name}」？该操作不可撤销。`)) return;
  try {
    await projectsStore.remove(project.id);
    if (selectedProjectId.value === project.id) {
      selectedProjectId.value = "";
    }
    // 刷新当前项目的会话列表，确保被删项目的会话缓存同步
    await conversationsStore.loadForProject(projectsStore.currentId || DEFAULT_PROJECT_ID);
    toast.success(`项目「${project.name}」已删除`);
  } catch {
    toast.error("删除项目失败");
  }
}

function goBack(): void {
  void router.push({ name: "ProjectChat", params: { projectId: "default" } });
}
</script>

<template>
  <div class="project-manager">
    <!-- 顶栏 -->
    <header class="pm-header">
      <button class="back-btn" type="button" @click="goBack">
        <ArrowLeft :size="18" aria-hidden="true" />
      </button>
      <h1 class="pm-title">项目管理</h1>
    </header>

    <div class="pm-body">
      <!-- 项目列表 -->
      <div class="pm-section">
        <div class="section-header">
          <h2 class="section-title">所有项目</h2>
          <button class="add-btn" type="button" @click="openCreate">
            <Plus :size="16" aria-hidden="true" />
            <span>新建</span>
          </button>
        </div>

        <!-- 默认项目 -->
        <div
          :class="['project-card', { 'project-card-active': selectedProjectId === '' || selectedProjectId === DEFAULT_PROJECT_ID }]"
          @click="selectProject(DEFAULT_PROJECT_ID)"
        >
          <Clipboard :size="20" class="project-icon-lucide" aria-hidden="true" />
          <div class="project-info">
            <span class="project-name">默认项目</span>
            <span class="project-desc">未分配项目的会话</span>
          </div>
        </div>

        <!-- 用户项目 -->
        <div
          v-for="proj in projectsStore.sortedProjects"
          :key="proj.id"
          :class="['project-card', { 'project-card-active': selectedProjectId === proj.id }]"
          @click="openEdit(proj)"
        >
          <component :is="resolveProjectIcon(proj.icon)" :size="20" class="project-icon-lucide" aria-hidden="true" />
          <div class="project-info">
            <span class="project-name">{{ proj.name }}</span>
            <span v-if="proj.description" class="project-desc">{{ proj.description }}</span>
            <span v-if="proj.workspace_path" class="project-meta"><FolderOpen :size="10" class="project-meta-icon" aria-hidden="true" /> {{ proj.workspace_path }}</span>
            <span v-if="proj.agents.length > 0" class="project-meta">{{ proj.agents.length }} 个 Agent</span>
          </div>
          <button
            class="project-delete"
            type="button"
            title="删除项目"
            aria-label="删除项目"
            @click.stop="handleDelete(proj)"
          >
            <Trash2 :size="14" aria-hidden="true" />
          </button>
        </div>
      </div>
    </div>

    <!-- 创建/编辑弹窗 -->
    <ProjectFormModal
      v-model="showModal"
      :mode="modalMode"
      :initial="editing"
      :agents="agentsStore.agents"
      @submit-create="handleCreate"
      @submit-edit="handleEdit"
    />
  </div>
</template>

<style scoped>
.project-manager {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--ip-color-bg-secondary);
}

.pm-header {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  height: var(--ip-spacing-12);
  padding: 0 var(--ip-spacing-5);
  border-bottom: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-header-backdrop);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  flex-shrink: 0;
}

.back-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: var(--ip-radius-md);
  background: transparent;
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.back-btn:hover {
  background: var(--ip-color-bg-hover);
  color: var(--ip-color-text-primary);
}

.pm-title {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.pm-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--ip-spacing-6);
  max-width: 720px;
  width: 100%;
  margin: 0 auto;
}

.pm-section {
  margin-bottom: var(--ip-spacing-8);
}

.section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--ip-spacing-3);
}

.section-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.add-btn {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  padding: var(--ip-spacing-1) var(--ip-spacing-3);
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-link);
  background: transparent;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.add-btn:hover {
  background: var(--ip-color-bg-hover);
  border-color: var(--ip-color-border-strong);
}

.project-card {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-3) var(--ip-spacing-4);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  margin-bottom: var(--ip-spacing-2);
  transition: var(--ip-transition-colors);
}

.project-card:hover {
  background: var(--ip-color-bg-hover);
}

.project-card-active {
  border-color: var(--ip-primary-500);
  background: var(--ip-primary-50);
}

[data-theme="dark"] .project-card-active {
  background: var(--ip-primary-900);
}

.project-icon-lucide {
  flex-shrink: 0;
  line-height: 1;
  color: var(--ip-color-text-secondary);
}

.project-meta-icon {
  display: inline-block;
  vertical-align: middle;
  flex-shrink: 0;
}

.project-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
  min-width: 0;
}

.project-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.project-desc {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.project-meta {
  font-size: 10px;
  color: var(--ip-color-text-tertiary);
  opacity: 0.8;
}

.project-delete {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition: var(--ip-transition-colors);
  opacity: 0;
}

.project-card:hover .project-delete {
  opacity: 1;
}

.project-delete:hover {
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
}
</style>
