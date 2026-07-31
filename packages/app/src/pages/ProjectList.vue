<script setup lang="ts">
// ProjectList.vue — 项目列表 + 内联新建项目表单
import { ref, reactive, onMounted } from "vue";
import { useRouter } from "vue-router";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/project";
import { useAgentStore } from "../stores/agent";
import type { NewProject, Project } from "../types";

const router = useRouter();
const project = useProjectStore();
const agent = useAgentStore();

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

function memberNames(p: Project): string {
  const ids = p.agents?.map((a) => a.agent_id) ?? [];
  const names = ids
    .map((id) => agent.getById(id)?.name)
    .filter(Boolean) as string[];
  if (names.length === 0) return "未分配成员";
  return names.join("、");
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
    const created = await project.create(input);
    isCreating.value = false;
    resetForm();
    // 创建后直接进入项目详情
    router.push(`/projects/${created.id}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : "创建项目失败";
  } finally {
    saving.value = false;
  }
}

async function deleteProject(p: Project) {
  if (!window.confirm(`确认删除项目「${p.name}」？\n项目内会话会变为散落会话（不会被删除）。`)) return;
  try {
    await project.remove(p.id);
  } catch (e) {
    console.error("删除项目失败:", e);
  }
}

onMounted(() => {
  project.load();
  agent.load();
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
            <div v-if="agent.list.length === 0" class="members-empty">暂无可用智能体，可稍后在项目详情里添加</div>
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

      <!-- 项目卡片 -->
      <div
        v-for="p in project.list"
        :key="p.id"
        class="proj-card"
        @click="router.push(`/projects/${p.id}`)"
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
          <svg class="card-chevron" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
          <button class="card-delete" title="删除项目" @click.stop="deleteProject(p)">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="3 6 5 6 21 6" /><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
          </button>
        </div>
      </div>

      <div v-if="project.loading && !project.list.length" class="loading-state">加载中...</div>
      <div v-else-if="project.list.length === 0" class="empty-hint">还没有项目，点上方「新建项目」创建</div>
    </div>
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

.card-delete {
  position: absolute; top: 10px; right: 12px;
  display: flex; align-items: center; justify-content: center;
  width: 26px; height: 26px; border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-disabled); background: none; border: none; cursor: pointer;
  opacity: 0; transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.proj-card:hover .card-delete { opacity: 1; }
.card-delete:hover { background: var(--ip-color-bg-tertiary); color: var(--ip-danger-text); }

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
</style>
