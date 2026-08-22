<script setup lang="ts">
// ProjectSettings.vue — 项目详情「设置」tab（MA-2 Commit 7）：三共享组件编排
// （ProjectBasicForm / ProjectMembersChips / ProjectContextEditor，与 ProjectList
// 展开区双入口复用）+ 归档入口。与展开区的差异只在容器：这里是页面级常驻区，
// 基础信息有显式保存/取消（表单脏检查），成员与项目背景仍是即时保存语义
// （组件内自带保存按钮/上交持久化，见 Commit 4 边界）。
import { computed, reactive, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useProjectStore } from "../../stores/project";
import { useChatStore } from "../../stores/chat";
import { bridge } from "../../api/bridge";
import ProjectBasicForm from "../../components/project/ProjectBasicForm.vue";
import ProjectMembersChips from "../../components/project/ProjectMembersChips.vue";
import ProjectContextEditor from "../../components/project/ProjectContextEditor.vue";

const route = useRoute();
const router = useRouter();
const project = useProjectStore();
const chat = useChatStore();

const projectId = computed(() => String(route.params.id ?? ""));
const current = computed(() => project.getById(projectId.value));

// ---- 基础信息表单（出生证：name/description/workspace）----
const editForm = reactive({
  name: "",
  description: "",
  workspacePath: "",
});
const editError = ref("");
const saving = ref(false);

function resetForm() {
  const p = current.value;
  editForm.name = p?.name ?? "";
  editForm.description = p?.description ?? "";
  editForm.workspacePath = p?.workspace_path ?? "";
  editError.value = "";
}
// keep-alive 按 :key=route.path 重建实例（项目+tab 维度），watch 兜底直链热替换等边缘
watch(projectId, resetForm, { immediate: true });

const dirty = computed(
  () =>
    editForm.name !== (current.value?.name ?? "") ||
    editForm.description !== (current.value?.description ?? "") ||
    editForm.workspacePath !== (current.value?.workspace_path ?? ""),
);

async function save() {
  if (!current.value || saving.value) return;
  if (!editForm.name.trim()) {
    editError.value = "项目名称不能为空";
    return;
  }
  saving.value = true;
  editError.value = "";
  try {
    await project.update({
      id: current.value.id,
      name: editForm.name.trim(),
      description: editForm.description.trim(),
      workspace_path: editForm.workspacePath.trim() || null,
      // 身份字段（头像/主题色已移除，avatar/theme_color 不进 payload——库存值原地保留）
    });
  } catch (e) {
    editError.value = e instanceof Error ? e.message : "保存失败";
  } finally {
    saving.value = false;
  }
}

// ---- 成员（即时持久化，与 ProjectList 展开区同款） ----
const memberIds = computed(() => (current.value?.agents ?? []).map((a) => a.agent_id));
async function addMember(agentId: string) {
  try {
    await bridge.projects.addAgent(projectId.value, agentId, "member");
    await project.load(true);
  } catch (e) {
    console.error("添加成员失败:", e);
  }
}
async function removeMember(agentId: string) {
  try {
    await bridge.projects.removeAgent(projectId.value, agentId);
    await project.load(true);
  } catch (e) {
    console.error("移除成员失败:", e);
  }
}

// ---- 归档（危险区：确认弹窗 → store.archive → 回项目列表） ----
const confirmArchive = ref(false);
const archiving = ref(false);
async function archive() {
  if (archiving.value) return;
  archiving.value = true;
  try {
    await project.archive(projectId.value);
    await chat.loadConversations(); // 会话从项目收起（侧栏缓存同步）
    router.push("/projects");
  } catch (e) {
    console.error("归档项目失败:", e);
    confirmArchive.value = false;
  } finally {
    archiving.value = false;
  }
}
</script>

<template>
  <div v-if="current" class="proj-settings">
    <section class="settings-card">
      <h3 class="card-title">基础信息</h3>
      <ProjectBasicForm
        :model-value="editForm"
        @update:model-value="Object.assign(editForm, $event)"
      />
      <div v-if="editError" class="form-error">{{ editError }}</div>
      <div class="form-actions">
        <button class="btn-link" :disabled="!dirty || saving" @click="resetForm">取消</button>
        <button class="btn btn-primary btn-sm" :disabled="!dirty || saving" @click="save">
          {{ saving ? "保存中" : "保存" }}
        </button>
      </div>
    </section>

    <section class="settings-card">
      <h3 class="card-title">成员</h3>
      <p class="card-hint">项目成员可被委派任务，也可在项目空间内开新会话（增删即时生效）</p>
      <ProjectMembersChips :member-ids="memberIds" @add="addMember" @remove="removeMember" />
    </section>

    <section class="settings-card">
      <h3 class="card-title">项目背景</h3>
      <p class="card-hint">project.md 随每轮对话注入本项目会话（system prompt），修改即时生效</p>
      <ProjectContextEditor :project-id="projectId" />
    </section>

    <section class="settings-card danger-zone">
      <h3 class="card-title">危险区</h3>
      <p class="card-hint">归档后项目及会话从列表收起，可随时在项目页「已归档」恢复</p>
      <button class="btn btn-danger btn-sm" @click="confirmArchive = true">归档项目</button>
    </section>

    <!-- 归档确认弹窗（ProjectList 同款视觉） -->
    <Transition name="overlay">
      <div v-if="confirmArchive" class="perm-overlay" @click.self="confirmArchive = false">
        <div class="perm-panel" @click.stop>
          <h3 class="perm-title">归档「{{ current.name }}」？</h3>
          <p class="perm-desc">项目及其会话会从列表收起，可随时在项目页「已归档」恢复。</p>
          <div class="perm-actions">
            <button class="btn-link" @click="confirmArchive = false">取消</button>
            <button class="btn btn-sm btn-confirm" :disabled="archiving" @click="archive">
              {{ archiving ? "归档中" : "确认归档" }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </div>
  <div v-else class="load-error">项目不存在或已删除。</div>
</template>

<style scoped>
/* 表单类内容限宽居中（GitHub/Linear settings 先例）：超宽窗口下卡片居中
   960px（比 720 舒展、不失扫视性）；窄窗口自然全宽。概览/轨迹 tab 保持全宽
   （表格列多，宽度是收益）。滚动条隐藏视觉、滚轮仍可滚（overflow 保留） */
.proj-settings {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-height: 0;
  width: 100%;
  max-width: 960px;
  margin: 0 auto;
  scrollbar-width: none; /* 标准 */
}
.proj-settings::-webkit-scrollbar { display: none; } /* Chromium/WebView2 */

.settings-card {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
  padding: 18px 20px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
}
.card-title {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.card-hint { margin: -6px 0 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.form-error { font-size: var(--ip-text-caption-size); color: var(--ip-danger-text); }
.form-actions { display: flex; align-items: center; justify-content: flex-end; gap: var(--ip-spacing-2); }

.btn {
  display: inline-flex; align-items: center; justify-content: center; gap: 6px;
  border-radius: var(--ip-radius-md); cursor: pointer; font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-sm { height: 30px; padding: 0 14px; font-size: var(--ip-text-body-sm-size); }
.btn:disabled { opacity: 0.5; cursor: not-allowed; }
.btn-primary { background-color: var(--ip-primary-500); color: white; }
.btn-primary:hover:not(:disabled) { background-color: var(--ip-primary-600); }
.btn-danger {
  align-self: flex-start;
  background: none; color: var(--ip-danger-text);
  border: 1px solid var(--ip-danger-border);
}
.btn-danger:hover:not(:disabled) { background-color: var(--ip-danger-bg); }
.btn-confirm { background-color: var(--ip-primary-500); color: white; }
.btn-link {
  height: 30px; padding: 0 8px; background: none; border: none; cursor: pointer;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); font-family: inherit;
}
.btn-link:hover:not(:disabled) { color: var(--ip-color-text-primary); }
.btn-link:disabled { opacity: 0.5; cursor: not-allowed; }

.load-error { padding: var(--ip-spacing-6); color: var(--ip-color-text-secondary); font-size: var(--ip-text-body-sm-size); }

/* 归档弹窗（ProjectList 同款） */
.perm-overlay {
  position: fixed; inset: 0; z-index: var(--ip-z-modal-overlay);
  background: rgba(0, 0, 0, 0.3);
  display: flex; align-items: center; justify-content: center;
}
.perm-panel {
  width: 380px; max-width: calc(100vw - 32px);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
  box-shadow: var(--ip-shadow-xl);
  padding: 20px;
  display: flex; flex-direction: column; gap: var(--ip-spacing-3);
}
.perm-title { margin: 0; font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.perm-desc { margin: 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); }
.perm-actions { display: flex; justify-content: flex-end; gap: var(--ip-spacing-2); margin-top: 4px; }

.overlay-enter-active, .overlay-leave-active { transition: opacity var(--ip-duration-base) var(--ip-ease-out); }
.overlay-enter-active .perm-panel, .overlay-leave-active .perm-panel { transition: transform var(--ip-duration-base) var(--ip-ease-out), opacity var(--ip-duration-base) var(--ip-ease-out); }
.overlay-enter-from, .overlay-leave-to { opacity: 0; }
.overlay-enter-from .perm-panel, .overlay-leave-to .perm-panel { opacity: 0; transform: scale(0.96) translateY(6px); }
</style>
