<script setup lang="ts">
// 项目管理页（Wave 2 重设计）
//
// 职责：
//   - 列出所有项目（含默认项目）— 网格布局（spec §2）
//   - 项目 CRUD：创建、编辑（基本信息 + 成员）、删除
//   - 搜索 / 过滤 / 排序（spec §2.4）
//   - 删除项目前的二次确认（保留旧行为）
//   - 网格响应式断点（spec §2.7）：
//       ≥ 1440px 4 列 / 1024-1439px 3 列 / 768-1023px 2 列 / < 768px 1 列
//
// 本页不再内联成员管理；统一由 ProjectFormModal 处理创建/编辑两态。
// 保留所有 emit / action：CRUD、排序、默认项目卡片（spec §2.8 行为保留清单）。

import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { Plus, ArrowLeft, Upload, Layers, LayoutGrid, List as ListIcon, Search } from "lucide-vue-next";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../stores/projects";
import { useConversationsStore } from "../stores/conversations";
import { useAgentsStore } from "../stores/agents";
import { useTemplatesStore } from "../stores/templates";
import { useToast } from "../composables/useToast";
import ProjectFormModal from "../components/project/ProjectFormModal.vue";
import ProjectCard from "../components/project/ProjectCard.vue";
import EmptyProjectCard from "../components/project/EmptyProjectCard.vue";
import { accentFromName, type ProjectAccent } from "../utils/projectAccent";
import type { Project, NewProject, ProjectMemberInput, ProjectPatch } from "../types";

const projectsStore = useProjectsStore();
const conversationsStore = useConversationsStore();
const agentsStore = useAgentsStore();
const templatesStore = useTemplatesStore();
const toast = useToast();
const route = useRoute();
const router = useRouter();

/** Modal 状态 */
const showModal = ref<boolean>(false);
const modalMode = ref<"create" | "edit">("create");
const editing = ref<Project | null>(null);

/** 当前选中的项目 ID（保留旧行为：用于详情区视觉聚焦） */
const selectedProjectId = ref<string>("");

/** 过滤 / 搜索 / 排序（spec §2.4） */
const filter = ref<"all" | "active" | "archived" | "template">("all");
const search = ref<string>("");
const sortBy = ref<"recent" | "name" | "created">("recent");
const view = ref<"card" | "list">("card");

onMounted(async () => {
  await projectsStore.loadAll();
  await agentsStore.ensureLoaded();
  await templatesStore.ensureLoaded();
  await loadAllConversations();
  const param = route.params.projectId as string | undefined;
  if (param && param !== "default") {
    selectedProjectId.value = param;
  }
});

/** 加载所有项目的会话（确保 stats 准确） */
async function loadAllConversations(): Promise<void> {
  // 默认项目
  try {
    await conversationsStore.loadForProject(DEFAULT_PROJECT_ID);
  } catch {
    /* ignore */
  }
  // 其他项目
  for (const p of projectsStore.projects) {
    try {
      await conversationsStore.loadForProject(p.id);
    } catch {
      /* ignore */
    }
  }
}

// ============================================================================
// 项目统计派生（spec §2.4）
// ============================================================================

interface ProjectStats {
  conversationCount: number;
  lastActiveAt: string | null;
  category: "system" | "active" | "archived" | "template";
  accent: ProjectAccent;
}

/** 每个项目的统计信息（含默认项目） */
const projectStats = computed<Record<string, ProjectStats>>(() => {
  const map: Record<string, ProjectStats> = {};

  // 默认项目
  const defaultConvs = conversationsStore.listForProject(DEFAULT_PROJECT_ID);
  map[DEFAULT_PROJECT_ID] = {
    conversationCount: defaultConvs.length,
    lastActiveAt: defaultConvs[0]?.updated_at ?? null,
    category: "system",
    accent: accentFromName("__default__"),
  };

  // 用户项目
  for (const p of projectsStore.sortedProjects) {
    const convs = conversationsStore.listForProject(p.id);
    map[p.id] = {
      conversationCount: convs.length,
      lastActiveAt: convs[0]?.updated_at ?? null,
      // Phase 2 当前无 archived / template 字段；全部归为 active（保留扩展空间）
      category: "active",
      accent: accentFromName(p.name),
    };
  }
  return map;
});

// ============================================================================
// filter / sort（spec §2.4）
// ============================================================================

/** 过滤：按 category */
function matchFilter(p: Project, f: typeof filter.value): boolean {
  // 默认项目始终保留在「全部 / active」中，不出现在 archived/template
  if (p.id === DEFAULT_PROJECT_ID) {
    return f === "all" || f === "active";
  }
  switch (f) {
    case "all":
      return true;
    case "active":
      return true; // Phase 2 无归档/模板字段，全部归 active
    case "archived":
      return false;
    case "template":
      return false;
    default:
      return true;
  }
}

/** 搜索：按 name + description */
function matchSearch(p: Project, q: string): boolean {
  if (!q.trim()) return true;
  const needle = q.trim().toLowerCase();
  return (
    p.name.toLowerCase().includes(needle) ||
    (p.description ?? "").toLowerCase().includes(needle)
  );
}

/** 排序：recent / name / created */
function matchSort(a: Project, b: Project, mode: typeof sortBy.value): number {
  switch (mode) {
    case "name":
      return a.name.localeCompare(b.name, "zh-CN");
    case "created":
      return (a.created_at ?? "").localeCompare(b.created_at ?? "");
    case "recent":
    default: {
      const sa = projectStats.value[a.id]?.lastActiveAt ?? "";
      const sb = projectStats.value[b.id]?.lastActiveAt ?? "";
      return sb.localeCompare(sa);
    }
  }
}

/** 默认项目卡片（始终顶部） */
const defaultProjectEntry = computed(() => ({
  id: DEFAULT_PROJECT_ID,
  name: "默认项目",
  description: "未分配项目的会话",
  icon: "folder",
  workspace_path: null,
  sort_order: -1,
  created_at: "",
  updated_at: "",
  agents: [],
}));

/** 过滤 + 排序后的项目列表（不含默认项目） */
const filteredUserProjects = computed(() => {
  return projectsStore.sortedProjects
    .filter((p) => matchFilter(p, filter.value))
    .filter((p) => matchSearch(p, search.value))
    .sort((a, b) => matchSort(a, b, sortBy.value));
});

/** 是否显示默认项目（在过滤 + 搜索结果中） */
const showDefaultProject = computed<boolean>(() => {
  return matchFilter(
    defaultProjectEntry.value as unknown as Project,
    filter.value,
  ) && matchSearch(defaultProjectEntry.value as unknown as Project, search.value);
});

/** 是否完全空（既无默认项目可见也无用户项目） */
const isEmpty = computed<boolean>(
  () => !showDefaultProject.value && filteredUserProjects.value.length === 0,
);

// ============================================================================
// Actions
// ============================================================================

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

function selectProject(p: Project): void {
  selectedProjectId.value = p.id;
  const routeParam = p.id === DEFAULT_PROJECT_ID ? "default" : p.id;
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
  if (!window.confirm(`确认删除项目「${project.name}」？该操作不可撤销。`)) return;
  try {
    await projectsStore.remove(project.id);
    if (selectedProjectId.value === project.id) {
      selectedProjectId.value = "";
    }
    // 刷新当前项目的会话列表，确保被删项目的会话缓存同步
    await conversationsStore.loadForProject(
      projectsStore.currentId || DEFAULT_PROJECT_ID,
    );
    toast.success(`项目「${project.name}」已删除`);
  } catch {
    toast.error("删除项目失败");
  }
}

function goBack(): void {
  void router.push({ name: "ProjectChat", params: { projectId: "default" } });
}

/** 导入项目（占位：Phase 2 暂未实现，保留按钮） */
function importProject(): void {
  toast.info("导入功能即将上线");
}

/** 从模板创建（占位：Phase 2 暂未实现，保留按钮） */
function createFromTemplate(): void {
  toast.info("从模板创建即将上线，可前往模板管理查看");
}

/** 列表视图下的删除 */
function listDelete(project: Project): void {
  void handleDelete(project);
}

/** 列表视图下的编辑 */
function listEdit(project: Project): void {
  openEdit(project);
}
</script>

<template>
  <div class="project-manager">
    <!-- 顶栏（保留） -->
    <header class="pm-header">
      <button class="back-btn" type="button" @click="goBack">
        <ArrowLeft :size="18" aria-hidden="true" />
      </button>
      <h1 class="pm-title">项目管理</h1>
    </header>

    <div class="pm-body">
      <!-- page-header（spec §2.2） -->
      <section class="pm-page-header">
        <div class="pm-page-header-text">
          <div class="page-eyebrow">项目是一等公民</div>
          <h1 class="page-title">管理你的 <em>项目空间</em></h1>
          <p class="page-sub">
            每个项目拥有独立的 Agent 团队、会话历史与工作目录。把同类任务归到一起，长链路工作更清晰。
          </p>
        </div>
        <div class="page-header-actions">
          <button class="btn btn-ghost btn-md" type="button" @click="importProject">
            <Upload :size="14" aria-hidden="true" />
            <span>导入</span>
          </button>
          <button
            class="btn btn-secondary btn-md"
            type="button"
            @click="createFromTemplate"
          >
            <Layers :size="14" aria-hidden="true" />
            <span>从模板创建</span>
          </button>
          <button class="btn btn-primary btn-md" type="button" @click="openCreate">
            <Plus :size="14" aria-hidden="true" />
            <span>新建项目</span>
          </button>
        </div>
      </section>

      <!-- toolbar（spec §2.2） -->
      <div class="toolbar">
        <div class="search">
          <Search :size="14" class="search-icon" aria-hidden="true" />
          <input
            v-model="search"
            type="search"
            class="search-input"
            placeholder="搜索项目名称或描述…"
            aria-label="搜索项目"
          />
        </div>
        <div class="filter-tabs" role="tablist" aria-label="项目过滤">
          <button
            v-for="opt in [
              { key: 'all', label: '全部' },
              { key: 'active', label: '活跃' },
              { key: 'archived', label: '已归档' },
              { key: 'template', label: '模板' },
            ]"
            :key="opt.key"
            type="button"
            role="tab"
            :aria-selected="filter === opt.key"
            :class="['filter-tab', { 'filter-tab--active': filter === opt.key }]"
            @click="filter = opt.key as typeof filter"
          >
            {{ opt.label }}
          </button>
        </div>
        <div class="toolbar-right">
          <label class="sort">
            <select v-model="sortBy" class="sort-select" aria-label="排序方式">
              <option value="recent">最近活跃</option>
              <option value="name">按名称</option>
              <option value="created">按创建时间</option>
            </select>
          </label>
          <div class="view-toggle" role="group" aria-label="视图切换">
            <button
              type="button"
              :class="['view-btn', { 'view-btn--active': view === 'card' }]"
              :aria-pressed="view === 'card'"
              title="卡片视图"
              @click="view = 'card'"
            >
              <LayoutGrid :size="14" aria-hidden="true" />
            </button>
            <button
              type="button"
              :class="['view-btn', { 'view-btn--active': view === 'list' }]"
              :aria-pressed="view === 'list'"
              title="列表视图"
              @click="view = 'list'"
            >
              <ListIcon :size="14" aria-hidden="true" />
            </button>
          </div>
        </div>
      </div>

      <!-- 主体：网格 or 列表 -->
      <!-- 网格视图 -->
      <section v-if="view === 'card'" class="project-grid">
        <!-- 默认项目卡片（始终顶部） -->
        <ProjectCard
          v-if="showDefaultProject"
          :project="defaultProjectEntry"
          :conversation-count="projectStats[DEFAULT_PROJECT_ID]?.conversationCount ?? 0"
          :last-active-at="projectStats[DEFAULT_PROJECT_ID]?.lastActiveAt ?? null"
          :category="projectStats[DEFAULT_PROJECT_ID]?.category ?? 'system'"
          :accent="projectStats[DEFAULT_PROJECT_ID]?.accent ?? 'glacier'"
          @click="selectProject"
          @edit="openEdit"
          @delete="handleDelete"
        />

        <!-- 用户项目 -->
        <ProjectCard
          v-for="(proj, idx) in filteredUserProjects"
          :key="proj.id"
          :project="proj"
          :conversation-count="projectStats[proj.id]?.conversationCount ?? 0"
          :last-active-at="projectStats[proj.id]?.lastActiveAt ?? null"
          :category="projectStats[proj.id]?.category ?? 'active'"
          :accent="projectStats[proj.id]?.accent ?? 'glacier'"
          :style="{ animationDelay: `${(showDefaultProject ? 1 : 0) * 60 + idx * 60}ms` }"
          class="project-grid-item"
          @click="selectProject"
          @edit="openEdit"
          @delete="handleDelete"
        />

        <!-- 空状态 -->
        <EmptyProjectCard v-if="isEmpty" />
      </section>

      <!-- 列表视图（保留旧行为：简单列表） -->
      <section v-else class="project-list">
        <div
          :class="[
            'project-row',
            { 'project-row--active': selectedProjectId === '' || selectedProjectId === DEFAULT_PROJECT_ID },
          ]"
          @click="selectProject(defaultProjectEntry as unknown as Project)"
        >
          <div class="project-row-name">默认项目</div>
          <div class="project-row-desc">未分配项目的会话</div>
          <div class="project-row-meta">{{ projectStats[DEFAULT_PROJECT_ID]?.conversationCount ?? 0 }} 会话</div>
          <button class="project-row-edit" type="button" @click.stop="openEdit(defaultProjectEntry as unknown as Project)">
            编辑
          </button>
        </div>
        <div
          v-for="proj in filteredUserProjects"
          :key="proj.id"
          :class="[
            'project-row',
            { 'project-row--active': selectedProjectId === proj.id },
          ]"
        >
          <div class="project-row-name" @click="selectProject(proj)">{{ proj.name }}</div>
          <div class="project-row-desc">{{ proj.description || '暂无描述' }}</div>
          <div class="project-row-meta">{{ projectStats[proj.id]?.conversationCount ?? 0 }} 会话 · {{ proj.agents.length }} Agent</div>
          <div class="project-row-actions">
            <button class="project-row-edit" type="button" @click="listEdit(proj)">编辑</button>
            <button class="project-row-delete" type="button" @click="listDelete(proj)">删除</button>
          </div>
        </div>
        <div v-if="filteredUserProjects.length === 0 && !showDefaultProject" class="project-list-empty">
          没有匹配的项目。
        </div>
      </section>
    </div>

    <!-- 创建/编辑弹窗（保留） -->
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
/* ============================================================
 * 根容器
 * ============================================================ */
.project-manager {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--ip-color-bg-primary);
  /* spec §2.5：页面背景渐变 */
  background-image: radial-gradient(
    ellipse 1000px 600px at 90% -10%,
    rgba(70, 128, 194, 0.05),
    transparent 60%
  );
  background-attachment: fixed;
}

/* ============================================================
 * 顶栏（保留旧行为）
 * ============================================================ */
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
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    color var(--ip-duration-base) var(--ip-ease-out);
}

.back-btn:hover {
  background: var(--ip-color-bg-hover);
  color: var(--ip-color-text-primary);
}

.pm-title {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

/* ============================================================
 * 主体
 * ============================================================ */
.pm-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--ip-spacing-8) var(--ip-spacing-6) var(--ip-spacing-12);
  width: 100%;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-6);
}

@media (min-width: 1440px) {
  .pm-body {
    max-width: 1280px;
  }
}

@media (max-width: 1023px) {
  .pm-body {
    padding: var(--ip-spacing-6) var(--ip-spacing-4) var(--ip-spacing-10);
  }
}

/* ============================================================
 * page-header（spec §2.2 + §2.5）
 * ============================================================ */
.pm-page-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--ip-spacing-6);
}

.pm-page-header-text {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  flex: 1;
  min-width: 0;
}

.page-eyebrow {
  font-size: 12px;
  color: var(--ip-primary-700);
  font-weight: var(--ip-font-weight-medium);
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.page-title {
  margin: 0;
  font-family: var(--ip-font-display);
  font-size: clamp(1.6rem, 2.6vw, 2rem);
  line-height: 1.15;
  letter-spacing: -0.02em;
  color: var(--ip-color-text-primary);
  font-weight: var(--ip-font-weight-semibold);
}

.page-title em {
  font-style: italic;
  color: var(--ip-primary-600);
}

.page-sub {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  max-width: 540px;
}

.page-header-actions {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  flex-shrink: 0;
}

/* ============================================================
 * button 重用样式（spec §2.2）
 * ============================================================ */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 0 var(--ip-spacing-4);
  height: 36px;
  border-radius: var(--ip-radius-md);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    border-color var(--ip-duration-base) var(--ip-ease-out),
    color var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out);
  border: 1px solid transparent;
  white-space: nowrap;
}

.btn-md {
  height: 36px;
  padding: 0 var(--ip-spacing-4);
}

.btn-primary {
  background: var(--ip-primary-500);
  color: var(--ip-white);
}

.btn-primary:hover {
  background: var(--ip-primary-600);
}

.btn-primary:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.btn-secondary {
  background: var(--ip-color-bg-elevated);
  color: var(--ip-color-text-primary);
  border-color: var(--ip-color-border-default);
}

.btn-secondary:hover {
  background: var(--ip-color-bg-hover);
  border-color: var(--ip-color-border-strong);
}

.btn-secondary:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.btn-ghost {
  background: transparent;
  color: var(--ip-color-text-secondary);
  border-color: transparent;
}

.btn-ghost:hover {
  background: var(--ip-color-bg-hover);
  color: var(--ip-color-text-primary);
}

.btn-ghost:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * toolbar（spec §2.2）
 * ============================================================ */
.toolbar {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  flex-wrap: wrap;
}

.search {
  position: relative;
  flex: 1 1 240px;
  min-width: 200px;
  max-width: 320px;
  display: inline-flex;
  align-items: center;
}

.search-icon {
  position: absolute;
  left: var(--ip-spacing-3);
  color: var(--ip-color-text-tertiary);
  pointer-events: none;
}

.search-input {
  width: 100%;
  height: 36px;
  padding: 0 var(--ip-spacing-3) 0 32px;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  transition:
    border-color var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out);
}

.search-input::placeholder {
  color: var(--ip-color-text-placeholder);
}

.search-input:focus {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.filter-tabs {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 3px;
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-md);
}

.filter-tab {
  border: none;
  background: transparent;
  padding: 0 var(--ip-spacing-3);
  height: 28px;
  border-radius: var(--ip-radius-sm);
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    color var(--ip-duration-base) var(--ip-ease-out);
  white-space: nowrap;
}

.filter-tab:hover {
  color: var(--ip-color-text-primary);
}

.filter-tab--active {
  background: var(--ip-color-bg-elevated);
  color: var(--ip-color-text-primary);
  box-shadow: var(--ip-shadow-xs);
}

.filter-tab:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.toolbar-right {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin-left: auto;
}

.sort {
  display: inline-flex;
  align-items: center;
}

.sort-select {
  height: 32px;
  padding: 0 var(--ip-spacing-2);
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition:
    border-color var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out);
}

.sort-select:hover {
  border-color: var(--ip-color-border-strong);
}

.sort-select:focus-visible {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.view-toggle {
  display: inline-flex;
  align-items: center;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  padding: 2px;
  gap: 2px;
}

.view-btn {
  border: none;
  background: transparent;
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--ip-radius-sm);
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    color var(--ip-duration-base) var(--ip-ease-out);
}

.view-btn:hover {
  background: var(--ip-color-bg-hover);
  color: var(--ip-color-text-secondary);
}

.view-btn--active {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}

.view-btn:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * project-grid（spec §2.5 + §2.7）
 * ============================================================ */
.project-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: var(--ip-spacing-4);
}

@media (min-width: 1440px) {
  .project-grid {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
}

@media (min-width: 1024px) and (max-width: 1439px) {
  .project-grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}

@media (min-width: 768px) and (max-width: 1023px) {
  .project-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 767px) {
  .project-grid {
    grid-template-columns: 1fr;
  }
}

/* 卡片入场 stagger */
.project-grid-item {
  animation: ip-empty-state-in var(--ip-duration-page) var(--ip-ease-emphasized) both;
  opacity: 0;
}

/* ============================================================
 * 列表视图（保留旧行为，简版）
 * ============================================================ */
.project-list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.project-row {
  display: grid;
  grid-template-columns: 1fr 2fr 1fr auto;
  align-items: center;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-3) var(--ip-spacing-4);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    border-color var(--ip-duration-base) var(--ip-ease-out);
}

.project-row:hover {
  background: var(--ip-color-bg-hover);
}

.project-row--active {
  border-color: var(--ip-primary-500);
  background: var(--ip-primary-50);
}

[data-theme="dark"] .project-row--active {
  background: var(--ip-primary-900);
}

.project-row-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.project-row-desc {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.project-row-meta {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}

.project-row-actions {
  display: inline-flex;
  gap: var(--ip-spacing-1);
}

.project-row-edit,
.project-row-delete {
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-elevated);
  padding: 0 var(--ip-spacing-2);
  height: 28px;
  border-radius: var(--ip-radius-sm);
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    color var(--ip-duration-base) var(--ip-ease-out);
}

.project-row-edit:hover,
.project-row-delete:hover {
  background: var(--ip-color-bg-hover);
}

.project-row-delete:hover {
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
  border-color: var(--ip-danger-border);
}

.project-list-empty {
  text-align: center;
  padding: var(--ip-spacing-12) var(--ip-spacing-6);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
}

/* ============================================================
 * 移动端响应式
 * ============================================================ */
@media (max-width: 767px) {
  .pm-page-header {
    flex-direction: column;
    align-items: stretch;
  }

  .page-header-actions {
    flex-wrap: wrap;
  }

  .toolbar {
    gap: var(--ip-spacing-2);
  }

  .search {
    flex: 1 1 100%;
    max-width: 100%;
  }

  .filter-tabs {
    overflow-x: auto;
    flex: 1 1 100%;
  }
}
</style>