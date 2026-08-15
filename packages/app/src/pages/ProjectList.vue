<script setup lang="ts">
// ProjectList.vue — 项目管理：列表 + 内联新建 + 内联编辑（点卡片展开配置）
import { ref, reactive, onMounted } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/project";
import { useAgentStore } from "../stores/agent";
import { useChatStore } from "../stores/chat";
import { bridge } from "../api/bridge";
import { formatTime, parseDbTime } from "../utils/time";
import type { NewProject, Project } from "../types";

const project = useProjectStore();
const agent = useAgentStore();
const chat = useChatStore();

// ===== 新建项目 =====
const isCreating = ref(false);
const saving = ref(false);
const error = ref("");

interface CreateForm {
  name: string;
  workspace_path: string;
  description: string;
  memberIds: Set<string>;
}
const form = reactive<CreateForm>({
  name: "",
  workspace_path: "",
  description: "",
  memberIds: new Set(),
});

function resetForm() {
  form.name = "";
  form.workspace_path = "";
  form.description = "";
  form.memberIds = new Set();
  error.value = "";
}

function toggleNew() {
  cancelEdit();
  resetForm();
  isCreating.value = !isCreating.value;
}

async function pickWorkspace() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择项目源码目录",
    defaultPath: form.workspace_path || undefined,
  });
  if (selected) form.workspace_path = selected;
}

function toggleMember(id: string) {
  if (form.memberIds.has(id)) form.memberIds.delete(id);
  else form.memberIds.add(id);
  // 触发响应式：重新赋值 Set
  form.memberIds = new Set(form.memberIds);
}

async function createProject() {
  if (saving.value) return;
  if (!form.name.trim()) {
    error.value = "项目名称不能为空";
    return;
  }
  saving.value = true;
  error.value = "";
  try {
    const input: NewProject = {
      name: form.name.trim(),
      description: form.description.trim() || undefined,
      workspace_path: form.workspace_path.trim() || undefined,
      agent_ids: [...form.memberIds],
    };
    await project.create(input);
    isCreating.value = false;
    resetForm();
  } catch (e) {
    error.value = e instanceof Error ? e.message : "创建项目失败";
  } finally {
    saving.value = false;
  }
}

// ===== 内联编辑（点卡片展开） =====
const expandedId = ref<string | null>(null);
const editName = ref("");
const editDesc = ref("");
const editWorkspace = ref("");
const editError = ref("");
const savingEdit = ref(false);

function toggleEdit(p: Project) {
  if (expandedId.value === p.id) {
    expandedId.value = null;
    return;
  }
  isCreating.value = false;
  expandedId.value = p.id;
  editName.value = p.name;
  editDesc.value = p.description || "";
  editWorkspace.value = p.workspace_path || "";
  editError.value = "";
}

function cancelEdit() {
  expandedId.value = null;
  editError.value = "";
}

async function pickEditWorkspace() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择项目源码目录",
    defaultPath: editWorkspace.value || undefined,
  });
  if (selected) editWorkspace.value = selected;
}

async function saveEdit(p: Project) {
  if (savingEdit.value) return;
  if (!editName.value.trim()) {
    editError.value = "项目名称不能为空";
    return;
  }
  savingEdit.value = true;
  editError.value = "";
  try {
    await project.update({
      id: p.id,
      name: editName.value.trim(),
      description: editDesc.value.trim(),
      workspace_path: editWorkspace.value.trim() || null,
    });
    expandedId.value = null;
  } catch (e) {
    editError.value = e instanceof Error ? e.message : "保存失败";
  } finally {
    savingEdit.value = false;
  }
}

function memberIdsOf(p: Project): string[] {
  return (p.agents ?? []).map((a) => a.agent_id);
}
function candidateAgents(p: Project) {
  const ids = new Set(memberIdsOf(p));
  return agent.list.filter((a) => !ids.has(a.id));
}
async function addMember(p: Project, agentId: string) {
  try {
    await bridge.projects.addAgent(p.id, agentId, "member");
    await project.load(true);
  } catch (e) {
    console.error("添加成员失败:", e);
  }
}
async function removeMember(p: Project, agentId: string) {
  try {
    await bridge.projects.removeAgent(p.id, agentId);
    await project.load(true);
  } catch (e) {
    console.error("移除成员失败:", e);
  }
}

// ===== 归档 / 恢复 / 永久删除 =====
const showArchived = ref(false);
const permTarget = ref<Project | null>(null);
const permMode = ref<"loose" | "delete">("loose");
const permDeleting = ref(false);
const confirmArchiveTarget = ref<Project | null>(null);

function convCountOf(p: Project | null): number {
  if (!p) return 0;
  return chat.conversations.filter((c) => c.project_id === p.id).length;
}

function archiveProject(p: Project) {
  confirmArchiveTarget.value = p;
}

async function confirmArchive() {
  const p = confirmArchiveTarget.value;
  if (!p) return;
  confirmArchiveTarget.value = null;
  if (expandedId.value === p.id) expandedId.value = null;
  try {
    await project.archive(p.id);
    await chat.loadConversations();
  } catch (e) {
    console.error("归档项目失败:", e);
  }
}

function cancelArchive() {
  confirmArchiveTarget.value = null;
}

async function unarchiveProject(p: Project) {
  try {
    await project.unarchive(p.id);
    await chat.loadConversations();
  } catch (e) {
    console.error("恢复项目失败:", e);
  }
}

function openPermDelete(p: Project) {
  permTarget.value = p;
  permMode.value = "loose";
}

async function confirmPermDelete() {
  const p = permTarget.value;
  if (!p || permDeleting.value) return;
  permDeleting.value = true;
  try {
    await project.permanentDelete(p.id, permMode.value === "delete");
    permTarget.value = null;
    await chat.loadConversations(); // 会话去向变更（转散落/已删），刷新缓存
  } catch (e) {
    console.error("永久删除项目失败:", e);
  } finally {
    permDeleting.value = false;
  }
}

function memberNames(p: Project): string {
  const names = memberIdsOf(p)
    .map((id) => agent.getById(id)?.name)
    .filter(Boolean) as string[];
  if (names.length === 0) return "未分配成员";
  return names.join("、");
}

// ===== MA-1：委派任务极简列表（入口保障——侧栏隐藏 delegation 会话后须有可达路径；
// MA-2 从事件日志派生完整台账/状态机，本轮只保「看得见 + 进得去」）=====
const expandedTasksId = ref<string | null>(null);

function toggleTasks(p: Project) {
  expandedTasksId.value = expandedTasksId.value === p.id ? null : p.id;
}

function delegationConvsOf(pid: string) {
  return chat.conversations
    .filter((c) => c.project_id === pid && c.kind === "delegation")
    .sort((a, b) => parseDbTime(b.updated_at).getTime() - parseDbTime(a.updated_at).getTime());
}

/** 任务状态点：bgStreams/激活流式=进行中（脉冲）；其余=已结束。
 *  done/failed 之分需要 turn_ended 派生（MA-2 台账），v1 不伪造。 */
function taskRunning(convId: string): boolean {
  return chat.streamingConvIds.has(convId);
}

function openDelegation(convId: string) {
  chat.openConversationAtTrajectory(convId);
}

onMounted(() => {
  project.load();
  agent.load();
  // 委派任务列表依赖会话缓存（侧栏常驻加载，此处兜底直入本页的场景）
  void chat.loadConversations();
});
</script>

<template>
  <div class="page-inner">
    <div class="content-header">
      <h2 class="content-title">项目</h2>
      <span class="header-hint">以项目隔离工作区、绑定源码目录、组织多 agent 协作</span>
    </div>

    <div class="proj-list">
      <!-- 新建项目卡片（虚线，仿侧栏「新建对话」） -->
      <div class="proj-card new-card" :class="{ expanded: isCreating }" @click="toggleNew">
        <div class="card-top">
          <div class="card-body">
            <div class="card-name-row">
              <svg class="new-plus" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
              </svg>
              <span class="card-name new-name">新建项目</span>
            </div>
            <div class="card-meta-row">
              <span class="new-hint">创建一个新的项目…</span>
            </div>
          </div>
          <svg class="card-chevron" :class="{ rotated: isCreating }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </div>

        <div v-if="isCreating" class="expand-panel" @click.stop>
          <div class="field">
            <label class="field-label">名称 <span class="req">*</span></label>
            <input v-model="form.name" type="text" class="input" placeholder="例如：ice-paw 桌面端" />
          </div>

          <div class="field">
            <label class="field-label">描述</label>
            <input v-model="form.description" type="text" class="input" placeholder="一句话说明项目用途（可选）" />
          </div>

          <div class="field">
            <label class="field-label">源码目录</label>
            <div class="workspace-group">
              <input
                v-model="form.workspace_path"
                type="text"
                class="input workspace-input"
                placeholder="选择项目源码根目录（agent 在此执行 git/read_file）"
                readonly
                @click="pickWorkspace"
              />
              <button type="button" class="ws-btn" title="选择目录" @click="pickWorkspace">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
              </button>
            </div>
            <p class="field-hint">绑定后，项目内会话的文件/代码工具会切换到此目录（取代 agent 个人工作区）</p>
          </div>

          <div class="field">
            <label class="field-label">成员 <span class="hint">可选，后续可增删</span></label>
            <div v-if="agent.list.length === 0" class="members-empty">暂无可用智能体，可创建后在项目卡片里添加</div>
            <div v-else class="member-chips">
              <button
                v-for="a in agent.list"
                :key="a.id"
                type="button"
                class="member-chip"
                :class="{ selected: form.memberIds.has(a.id) }"
                @click="toggleMember(a.id)"
              >
                {{ a.name }}
              </button>
            </div>
          </div>

          <div v-if="error" class="form-error">{{ error }}</div>

          <div class="form-actions">
            <button class="btn-link" @click="toggleNew">取消</button>
            <button class="btn btn-primary btn-sm" :disabled="saving" @click="createProject">
              {{ saving ? "创建中" : "创建项目" }}
            </button>
          </div>
        </div>
      </div>

      <div class="list-divider"></div>

      <!-- 项目卡片（点击内联展开编辑） -->
      <div
        v-for="p in project.activeProjects"
        :key="p.id"
        class="proj-card"
        :class="{ expanded: expandedId === p.id }"
        @click="toggleEdit(p)"
      >
        <div class="card-top">
          <div class="card-avatar">{{ p.name.charAt(0) }}</div>
          <div class="card-body">
            <div class="card-name-row">
              <span class="card-name">{{ p.name }}</span>
              <span v-if="p.workspace_path" class="card-tag">已绑定目录</span>
            </div>
            <div class="card-meta-row">
              <span v-if="p.workspace_path" class="card-workspace" :title="p.workspace_path">{{ p.workspace_path }}</span>
              <span v-else class="card-workspace none">未绑定源码目录</span>
            </div>
            <div class="card-meta-row">
              <span class="card-members">{{ memberNames(p) }}</span>
            </div>
          </div>
          <svg class="card-chevron" :class="{ rotated: expandedId === p.id }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
          <button class="card-action" title="归档项目" @click.stop="archiveProject(p)">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="21 8 21 21 3 21 3 8" /><rect x="1" y="3" width="22" height="5" /><line x1="10" y1="12" x2="14" y2="12" />
            </svg>
          </button>
        </div>

        <!-- MA-1：委派任务（项目维度入口——delegation 后台子会话不在侧栏，此处保「看得见 + 进得去」） -->
        <div v-if="delegationConvsOf(p.id).length > 0" class="tasks-block" @click.stop>
          <button class="tasks-toggle" @click="toggleTasks(p)">
            <svg class="tasks-chev" :class="{ rotated: expandedTasksId === p.id }" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6" /></svg>
            <span>委派任务</span>
            <span class="tasks-count">{{ delegationConvsOf(p.id).length }}</span>
            <span v-if="delegationConvsOf(p.id).some((c) => taskRunning(c.id))" class="tasks-live">进行中</span>
          </button>
          <div v-if="expandedTasksId === p.id" class="tasks-list">
            <button
              v-for="c in delegationConvsOf(p.id)"
              :key="c.id"
              class="task-item"
              :title="taskRunning(c.id) ? '进行中，点击查看轨迹' : '已结束，点击查看轨迹'"
              @click="openDelegation(c.id)"
            >
              <span class="task-dot" :class="{ running: taskRunning(c.id) }"></span>
              <span class="task-title">{{ c.title || "委派任务" }}</span>
              <span class="task-agent">{{ agent.getById(c.agent_id)?.name || "" }}</span>
              <span class="task-time">{{ formatTime(c.updated_at) }}</span>
            </button>
          </div>
        </div>

        <!-- 展开态：内联编辑配置（不跳页、不切换对话空间） -->
        <div v-if="expandedId === p.id" class="expand-panel" @click.stop>
          <div class="field">
            <label class="field-label">名称 <span class="req">*</span></label>
            <input v-model="editName" type="text" class="input" placeholder="项目名称" />
          </div>

          <div class="field">
            <label class="field-label">描述</label>
            <input v-model="editDesc" type="text" class="input" placeholder="一句话说明项目用途（可选）" />
          </div>

          <div class="field">
            <label class="field-label">源码目录</label>
            <div class="workspace-group">
              <input v-model="editWorkspace" type="text" class="input workspace-input" placeholder="选择项目源码根目录（可选）" readonly @click="pickEditWorkspace" />
              <button type="button" class="ws-btn" title="选择目录" @click="pickEditWorkspace">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
              </button>
            </div>
            <p class="field-hint">绑定后，项目内会话的文件/代码工具切换到此目录；留空则回退 agent 工作区</p>
          </div>

          <div class="field">
            <label class="field-label">成员</label>
            <div v-if="memberIdsOf(p).length === 0 && candidateAgents(p).length === 0" class="members-empty">暂无可用智能体</div>
            <div v-else class="member-chips">
              <button
                v-for="m in memberIdsOf(p)"
                :key="'m-' + m"
                type="button"
                class="member-chip selected"
                :title="`移除 ${agent.getById(m)?.name ?? ''}`"
                @click="removeMember(p, m)"
              >× {{ agent.getById(m)?.name ?? '未知' }}</button>
              <button
                v-for="a in candidateAgents(p)"
                :key="'a-' + a.id"
                type="button"
                class="member-chip"
                :title="`添加 ${a.name}`"
                @click="addMember(p, a.id)"
              >+ {{ a.name }}</button>
            </div>
          </div>

          <div v-if="editError" class="form-error">{{ editError }}</div>

          <div class="form-actions">
            <button class="btn-link" @click="cancelEdit">取消</button>
            <button class="btn btn-primary btn-sm" :disabled="savingEdit" @click="saveEdit(p)">
              {{ savingEdit ? "保存中" : "保存" }}
            </button>
          </div>
        </div>
      </div>

      <div v-if="project.loading && !project.list.length" class="loading-state">加载中...</div>
      <div v-else-if="project.activeProjects.length === 0" class="empty-hint">
        {{ project.archivedProjects.length ? '活跃项目为空（已归档见下方）' : '还没有项目，点上方「新建项目」创建' }}
      </div>

      <!-- 已归档项目（默认折叠） -->
      <div v-if="project.archivedProjects.length > 0" class="archive-section">
        <button class="archive-header" @click="showArchived = !showArchived">
          <svg class="archive-chev" :class="{ rotated: showArchived }" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6" /></svg>
          <span class="archive-title">已归档</span>
          <span class="archive-count">{{ project.archivedProjects.length }}</span>
        </button>
        <div v-if="showArchived" class="archive-list">
          <div v-for="p in project.archivedProjects" :key="p.id" class="archive-row">
            <span class="archive-name">{{ p.name }}</span>
            <div class="archive-actions">
              <button class="archive-btn" @click="unarchiveProject(p)">恢复</button>
              <button class="archive-btn danger" @click="openPermDelete(p)">永久删除</button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 归档确认弹窗（替代 window.confirm，统一视觉风格） -->
    <Transition name="overlay">
      <div v-if="confirmArchiveTarget" class="perm-overlay" @click.self="cancelArchive">
        <div class="perm-panel" @click.stop>
          <h3 class="perm-title">归档「{{ confirmArchiveTarget.name }}」？</h3>
          <p class="perm-desc">项目及其会话会从列表收起，可随时在「已归档」恢复。</p>
          <div class="perm-actions">
            <button class="btn-link" @click="cancelArchive">取消</button>
            <button class="btn btn-sm" style="background: var(--ip-primary-500); color: white;" @click="confirmArchive">确认归档</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 永久删除确认弹窗 -->
    <Transition name="overlay">
      <div v-if="permTarget" class="perm-overlay" @click.self="permTarget = null">
        <div class="perm-panel" @click.stop>
          <h3 class="perm-title">永久删除「{{ permTarget.name }}」？</h3>
          <p class="perm-desc">此操作不可恢复。该项目内有 {{ convCountOf(permTarget) }} 个会话：</p>
          <div class="perm-options">
            <label class="perm-option"><input v-model="permMode" type="radio" value="loose" /> 转为散落会话（保留记录）</label>
            <label class="perm-option"><input v-model="permMode" type="radio" value="delete" /> 连同这些会话一起删除</label>
          </div>
          <div class="perm-actions">
            <button class="btn-link" @click="permTarget = null">取消</button>
            <button class="btn btn-danger btn-sm" :disabled="permDeleting" @click="confirmPermDelete">{{ permDeleting ? "删除中" : "确认永久删除" }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.page-inner { flex: 1; display: flex; flex-direction: column; padding: 0; min-height: 0; }

.content-header {
  display: flex; align-items: baseline; gap: 12px;
  padding: 20px 28px 0; flex-shrink: 0; min-height: 56px;
}
.content-title {
  font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin: 0;
}
.header-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

/* ===== 列表 ===== */
.proj-list {
  flex: 1; overflow-y: auto; padding: 8px 28px 24px;
  display: flex; flex-direction: column; gap: 8px; min-height: 0;
}

/* ===== 卡片 ===== */
.proj-card {
  position: relative;
  padding: 14px 16px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.proj-card:hover { border-color: var(--ip-primary-300); box-shadow: var(--ip-shadow-sm); }
.proj-card.expanded { border-color: var(--ip-primary-400); box-shadow: var(--ip-shadow-sm); }

.new-card { border: 1px dashed var(--ip-color-border-default); background-color: transparent; }
.new-card:hover { border-color: var(--ip-primary-400); background-color: var(--ip-color-bg-tertiary); }
.new-card.expanded { border-style: solid; border-color: var(--ip-primary-400); background-color: var(--ip-color-bg-secondary); }

.list-divider { height: 1px; background-color: var(--ip-color-border-default); margin: 2px 4px; }

.card-top { display: flex; align-items: center; gap: 12px; }

.card-avatar {
  width: 36px; height: 36px; border-radius: var(--ip-radius-md);
  background: linear-gradient(135deg, var(--ip-primary-400), var(--ip-primary-600));
  color: white; display: flex; align-items: center; justify-content: center;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
  flex-shrink: 0;
}
.new-plus { flex-shrink: 0; color: var(--ip-color-primary-tint-text); }

.card-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }

.card-name-row { display: flex; align-items: center; gap: 6px; }
.card-name {
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.new-name { color: var(--ip-color-primary-tint-text); }
.new-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); padding-left: 22px; }

.card-tag {
  flex-shrink: 0; font-size: 10px;
  color: var(--ip-color-primary-tint-text); background: var(--ip-color-primary-tint-bg);
  padding: 0 6px; line-height: 18px; border-radius: var(--ip-radius-full);
}

.card-meta-row {
  display: flex; align-items: center; gap: 6px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary);
}
.card-workspace {
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-family: var(--ip-font-mono);
}
.card-workspace.none { color: var(--ip-color-text-disabled); font-style: italic; font-family: inherit; }
.card-members { color: var(--ip-color-text-tertiary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.card-chevron {
  flex-shrink: 0; color: var(--ip-color-text-disabled);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.card-chevron.rotated { transform: rotate(90deg); color: var(--ip-primary-600); }

.card-action {
  position: absolute; top: 10px; right: 12px;
  display: flex; align-items: center; justify-content: center;
  width: 26px; height: 26px; border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-disabled); background: none; border: none; cursor: pointer;
  opacity: 0; transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.proj-card:hover .card-action { opacity: 1; }
.card-action:hover { background: var(--ip-color-bg-tertiary); color: var(--ip-primary-600); }

/* ===== 展开面板（新建表单） ===== */
.expand-panel {
  margin-top: 12px; padding-top: 12px;
  border-top: 1px solid var(--ip-color-border-default);
  display: flex; flex-direction: column; gap: 14px;
}

.field { display: flex; flex-direction: column; gap: 6px; }
.field-label {
  font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  display: flex; align-items: center; gap: 6px;
}
.req { color: var(--ip-danger-text); }
.hint { color: var(--ip-color-text-tertiary); font-weight: var(--ip-font-weight-regular); }

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

.members-empty { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.member-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.member-chip {
  height: 28px; padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-full); cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.member-chip:hover { border-color: var(--ip-primary-300); }
.member-chip.selected {
  color: var(--ip-color-primary-tint-text); background-color: var(--ip-color-primary-tint-bg);
  border-color: var(--ip-primary-400);
}

.form-error { font-size: var(--ip-text-caption-size); color: var(--ip-danger-text); }

.form-actions { display: flex; align-items: center; justify-content: flex-end; gap: 8px; }

.btn {
  display: inline-flex; align-items: center; justify-content: center; gap: 6px;
  border-radius: var(--ip-radius-md); cursor: pointer; font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-sm { height: 30px; padding: 0 14px; font-size: var(--ip-text-body-sm-size); }
.btn-primary { background-color: var(--ip-primary-500); color: white; }
.btn-primary:hover:not(:disabled) { background-color: var(--ip-primary-600); }
.btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
.btn-link {
  height: 30px; padding: 0 8px; background: none; border: none; cursor: pointer;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary);
  font-family: inherit;
}
.btn-link:hover { color: var(--ip-color-text-primary); }

/* ===== 状态 ===== */
.loading-state { padding: 20px; text-align: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }
.empty-hint { padding: 16px 12px; text-align: center; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

/* ===== 已归档区 ===== */
.archive-section { margin-top: 8px; display: flex; flex-direction: column; gap: 2px; }
.archive-header {
  display: flex; align-items: center; gap: 6px;
  padding: 8px 8px;
  color: var(--ip-color-text-tertiary);
  background: none; border: none; cursor: pointer; font-family: inherit;
  font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-semibold);
  border-radius: var(--ip-radius-md);
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.archive-header:hover { color: var(--ip-color-text-secondary); }
.archive-chev { transition: transform var(--ip-duration-fast) var(--ip-ease-out); }
.archive-chev.rotated { transform: rotate(90deg); }
.archive-count {
  font-size: 10px; line-height: 16px; padding: 0 5px;
  color: var(--ip-color-text-tertiary); background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-full);
}
.archive-list { display: flex; flex-direction: column; gap: 2px; padding-left: 4px; }
.archive-row {
  display: flex; align-items: center; gap: 8px;
  padding: 7px 10px;
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-md);
}
.archive-name {
  flex: 1; min-width: 0;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.archive-actions { display: flex; gap: 4px; flex-shrink: 0; }
.archive-btn {
  height: 24px; padding: 0 10px;
  font-size: var(--ip-text-caption-size); font-family: inherit;
  color: var(--ip-color-text-secondary);
  background: none; border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md); cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.archive-btn:hover { color: var(--ip-color-text-primary); border-color: var(--ip-primary-400); }
.archive-btn.danger:hover { color: var(--ip-danger-text); border-color: var(--ip-danger-border); }

/* ===== 永久删除弹窗 ===== */
.perm-overlay {
  position: fixed; inset: 0; z-index: var(--ip-z-modal-overlay);
  background: rgba(0,0,0,0.3);
  display: flex; align-items: center; justify-content: center;
}
.perm-panel {
  width: 380px; max-width: calc(100vw - 32px);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
  box-shadow: var(--ip-shadow-xl);
  padding: 20px;
  display: flex; flex-direction: column; gap: 12px;
}
.perm-title { margin: 0; font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.perm-desc { margin: 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); }
.perm-options { display: flex; flex-direction: column; gap: 8px; }
.perm-option {
  display: flex; align-items: center; gap: 8px;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-primary);
  cursor: pointer;
}
.perm-option input { accent-color: var(--ip-primary-500); }
.perm-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }

.btn-danger { background-color: var(--ip-danger-base); color: white; }
.btn-danger:hover:not(:disabled) { opacity: 0.9; }
.btn-danger:disabled { opacity: 0.5; cursor: not-allowed; }

/* 弹窗过渡 */
.overlay-enter-active, .overlay-leave-active { transition: opacity var(--ip-duration-base) var(--ip-ease-out); }
.overlay-enter-active .perm-panel, .overlay-leave-active .perm-panel { transition: transform var(--ip-duration-base) var(--ip-ease-out), opacity var(--ip-duration-base) var(--ip-ease-out); }
.overlay-enter-from, .overlay-leave-to { opacity: 0; }
.overlay-enter-from .perm-panel, .overlay-leave-to .perm-panel { opacity: 0; transform: scale(0.96) translateY(6px); }

/* ===== MA-1：委派任务（项目卡内极简列表，v1）===== */
.tasks-block {
  margin-top: 10px; padding-top: 8px;
  border-top: 1px dashed var(--ip-color-border-default);
}
.tasks-toggle {
  display: flex; align-items: center; gap: 6px;
  background: none; border: none; padding: 0; cursor: pointer;
  font-family: inherit; font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.tasks-toggle:hover { color: var(--ip-color-text-secondary); }
.tasks-chev { transition: transform var(--ip-duration-fast) var(--ip-ease-out); }
.tasks-chev.rotated { transform: rotate(90deg); }
.tasks-count {
  font-size: 10px; line-height: 16px; padding: 0 5px;
  color: var(--ip-color-text-tertiary); background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-full);
}
/* 「进行中」徽章走 tint 令牌（soft 系），不直接用 primary 底色 */
.tasks-live {
  font-size: 10px; line-height: 16px; padding: 0 6px;
  color: var(--ip-color-primary-tint-text); background: var(--ip-color-primary-tint-bg);
  border-radius: var(--ip-radius-full);
}
.tasks-list { margin-top: 6px; display: flex; flex-direction: column; gap: 2px; }
.task-item {
  display: flex; align-items: center; gap: 8px;
  padding: 6px 8px; border: none; border-radius: var(--ip-radius-md);
  background-color: var(--ip-color-bg-tertiary); cursor: pointer;
  font-family: inherit; text-align: left;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.task-item:hover { background-color: var(--ip-primary-soft-bg, var(--ip-color-bg-secondary)); }
/* 状态点诚实原则：只区分「进行中」（脉冲）与「已结束」（中性）；
   done/failed 之分需 turn_ended 派生（MA-2 台账），v1 不伪造。 */
.task-dot { width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0; background: var(--ip-color-text-disabled); }
.task-dot.running { background: var(--ip-primary-500); animation: task-dot-pulse 1.2s ease-in-out infinite; }
@keyframes task-dot-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
.task-title {
  flex: 1; min-width: 0;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.task-agent { flex-shrink: 0; font-size: 11px; color: var(--ip-primary-600); font-weight: var(--ip-font-weight-medium); }
.task-time { flex-shrink: 0; font-size: 11px; color: var(--ip-color-text-disabled); font-variant-numeric: tabular-nums; }
</style>
