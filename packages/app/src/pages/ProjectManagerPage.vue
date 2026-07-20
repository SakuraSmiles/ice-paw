<script setup lang="ts">
// 项目管理页（Phase 2）
//
// 职责：
//   - 列出所有项目（含默认项目）
//   - 项目 CRUD：创建、编辑名称/描述、删除
//   - 项目成员管理：添加/移除 Agent

import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../stores/projects";
import { useAgentsStore } from "../stores/agents";
import { useToast } from "../composables/useToast";
import { Plus, Trash2, FolderClosed, ArrowLeft, UserPlus, X } from "lucide-vue-next";
import type { Project } from "../types";

const projectsStore = useProjectsStore();
const agentsStore = useAgentsStore();
const toast = useToast();
const route = useRoute();
const router = useRouter();

/** 是否显示新建表单 */
const showCreate = ref(false);
const newName = ref("");
const newDesc = ref("");

/** 当前选中的项目 ID（从路由参数获取） */
const selectedProjectId = ref<string>("");

/** 从路由参数初始化 */
onMounted(async () => {
  await projectsStore.loadAll();
  await agentsStore.ensureLoaded();
  const param = route.params.projectId as string | undefined;
  if (param && param !== "default") {
    selectedProjectId.value = param;
  }
});

/** 选中的项目实体 */
const selectedProject = computed<Project | null>(() => {
  if (!selectedProjectId.value || selectedProjectId.value === DEFAULT_PROJECT_ID) {
    return null;
  }
  return projectsStore.projects.find((p) => p.id === selectedProjectId.value) ?? null;
});

/** 选中项目 */
function selectProject(id: string): void {
  selectedProjectId.value = id;
  const routeParam = id === DEFAULT_PROJECT_ID ? "default" : id;
  void router.replace({ name: "ProjectSettings", params: { projectId: routeParam } });
}

/** 创建项目 */
async function handleCreate(): Promise<void> {
  const name = newName.value.trim();
  if (!name) {
    toast.warning("项目名称不能为空");
    return;
  }
  try {
    const created = await projectsStore.create({ name, description: newDesc.value.trim() || undefined });
    selectedProjectId.value = created.id;
    showCreate.value = false;
    newName.value = "";
    newDesc.value = "";
    toast.success(`项目「${created.name}」已创建`);
  } catch {
    toast.error("创建项目失败");
  }
}

/** 删除项目 */
async function handleDelete(project: Project): Promise<void> {
  if (project.id === DEFAULT_PROJECT_ID) {
    toast.warning("默认项目无法删除");
    return;
  }
  try {
    await projectsStore.remove(project.id);
    if (selectedProjectId.value === project.id) {
      selectedProjectId.value = "";
    }
    toast.success(`项目「${project.name}」已删除`);
  } catch {
    toast.error("删除项目失败");
  }
}

/** 添加 Agent 到项目 */
async function handleAddAgent(project: Project, agentId: string): Promise<void> {
  try {
    await projectsStore.addAgent(project.id, agentId, "member");
    toast.success("Agent 已添加到项目");
  } catch {
    toast.error("添加 Agent 失败");
  }
}

/** 从项目移除 Agent */
async function handleRemoveAgent(project: Project, agentId: string): Promise<void> {
  try {
    await projectsStore.removeAgent(project.id, agentId);
    toast.success("Agent 已从项目移除");
  } catch {
    toast.error("移除 Agent 失败");
  }
}

/** 获取不属于当前选中项目的 Agent 列表 */
function availableAgents(project: Project | null): typeof agentsStore.agents {
  if (!project) return [];
  const memberIds = new Set(project.agents.map((a) => a.agent_id));
  return agentsStore.agents.filter((a) => !memberIds.has(a.id));
}

/** 返回聊天页 */
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
          <button
            v-if="!showCreate"
            class="add-btn"
            type="button"
            @click="showCreate = true"
          >
            <Plus :size="16" aria-hidden="true" />
            <span>新建</span>
          </button>
        </div>

        <!-- 新建表单 -->
        <div v-if="showCreate" class="create-form">
          <input
            v-model="newName"
            class="form-input"
            type="text"
            placeholder="项目名称"
            @keyup.enter="handleCreate"
            @keyup.escape="showCreate = false"
          />
          <input
            v-model="newDesc"
            class="form-input"
            type="text"
            placeholder="描述（可选）"
            @keyup.enter="handleCreate"
            @keyup.escape="showCreate = false"
          />
          <div class="form-actions">
            <button class="btn btn-cancel" type="button" @click="showCreate = false">取消</button>
            <button class="btn btn-confirm" type="button" @click="handleCreate">创建</button>
          </div>
        </div>

        <!-- 默认项目 -->
        <div
          :class="['project-card', { 'project-card-active': selectedProjectId === '' || selectedProjectId === DEFAULT_PROJECT_ID }]"
          @click="selectProject(DEFAULT_PROJECT_ID)"
        >
          <span class="project-icon-emoji">📋</span>
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
          @click="selectProject(proj.id)"
        >
          <FolderClosed :size="20" class="project-icon" aria-hidden="true" />
          <div class="project-info">
            <span class="project-name">{{ proj.name }}</span>
            <span v-if="proj.description" class="project-desc">{{ proj.description }}</span>
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

      <!-- 选中项目详情 -->
      <div v-if="selectedProject" class="pm-section">
        <h2 class="section-title">{{ selectedProject.name }} — 成员管理</h2>

        <!-- 当前成员 -->
        <div v-if="selectedProject.agents.length > 0" class="member-list">
          <div
            v-for="member in selectedProject.agents"
            :key="member.agent_id"
            class="member-item"
          >
            <span class="member-name">{{ agentsStore.byId(member.agent_id)?.name ?? member.agent_id }}</span>
            <span class="member-role">{{ member.role }}</span>
            <button
              class="member-remove"
              type="button"
              title="移除"
              @click="handleRemoveAgent(selectedProject, member.agent_id)"
            >
              <X :size="14" aria-hidden="true" />
            </button>
          </div>
        </div>
        <div v-else class="empty-text">暂无 Agent 成员</div>

        <!-- 添加 Agent -->
        <div v-if="availableAgents(selectedProject).length > 0" class="add-agent-row">
          <UserPlus :size="16" class="add-agent-icon" aria-hidden="true" />
          <select
            class="agent-select"
            @change="(e) => { const target = e.target as HTMLSelectElement; if (target.value && selectedProject) handleAddAgent(selectedProject, target.value); target.value = ''; }"
          >
            <option value="">添加 Agent…</option>
            <option
              v-for="agent in availableAgents(selectedProject)"
              :key="agent.id"
              :value="agent.id"
            >
              {{ agent.name }}
            </option>
          </select>
        </div>
      </div>
    </div>
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

.create-form {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-4);
  margin-bottom: var(--ip-spacing-3);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
}

.form-input {
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  outline: none;
  transition: var(--ip-transition-colors);
}

.form-input:focus {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.form-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--ip-spacing-2);
}

.btn {
  padding: var(--ip-spacing-1) var(--ip-spacing-4);
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.btn-cancel {
  background: var(--ip-color-bg-primary);
  color: var(--ip-color-text-secondary);
  border: 1px solid var(--ip-color-border-default);
}

.btn-cancel:hover {
  background: var(--ip-color-bg-hover);
}

.btn-confirm {
  background: var(--ip-primary-500);
  color: white;
  border: 1px solid var(--ip-primary-500);
}

.btn-confirm:hover {
  background: var(--ip-primary-600);
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

.project-icon-emoji {
  font-size: 20px;
  flex-shrink: 0;
  line-height: 1;
}

.project-icon {
  flex-shrink: 0;
  color: var(--ip-color-text-secondary);
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

.member-list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.member-item {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
}

.member-name {
  flex: 1;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}

.member-role {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  text-transform: uppercase;
}

.member-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.member-remove:hover {
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
}

.empty-text {
  padding: var(--ip-spacing-4);
  text-align: center;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
}

.add-agent-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin-top: var(--ip-spacing-3);
}

.add-agent-icon {
  color: var(--ip-color-text-tertiary);
}

.agent-select {
  flex: 1;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  outline: none;
}
</style>
