<script setup lang="ts">
// Agent 管理页
//
// 职责：
//   - 顶部「+ 新建 Agent」按钮
//   - Agent 卡片列表（v-for AgentCard）
//   - 无数据时显示 EmptyAgentHint
//   - 挂载 AgentForm（侧滑面板）+ ConfirmDeleteDialog（确认弹窗）
//   - 处理创建/编辑/删除的全部业务逻辑

import { ref, onMounted } from "vue";
import { useAgentsStore } from "../stores/agents";
import { useToast } from "../composables/useToast";
import type { Agent } from "../types";
import type { AgentFormPayload } from "../components/agent/AgentForm.vue";
import AgentCard from "../components/agent/AgentCard.vue";
import AgentForm from "../components/agent/AgentForm.vue";
import ConfirmDeleteDialog from "../components/agent/ConfirmDeleteDialog.vue";
import EmptyAgentHint from "../components/agent/EmptyAgentHint.vue";

const agentsStore = useAgentsStore();
const toast = useToast();

// ============================================================================
// 表单状态
// ============================================================================

/** 是否显示创建/编辑表单面板 */
const formOpen = ref(false);

/** 表单模式：新建或编辑 */
const formMode = ref<"create" | "edit">("create");

/** 编辑模式下目标 Agent */
const editingAgent = ref<Agent | null>(null);

// ============================================================================
// 删除确认弹窗状态
// ============================================================================

const deleteDialogOpen = ref(false);
const deleteTarget = ref<Agent | null>(null);

// ============================================================================
// 生命周期
// ============================================================================

onMounted(() => {
  // 页面挂载时确保 Agent 列表已加载
  agentsStore.ensureLoaded();
});

// ============================================================================
// 创建
// ============================================================================

/** 打开创建表单 */
function openCreate(): void {
  formMode.value = "create";
  editingAgent.value = null;
  formOpen.value = true;
}

// ============================================================================
// 编辑
// ============================================================================

/** 打开编辑表单 */
function openEdit(agent: Agent): void {
  formMode.value = "edit";
  editingAgent.value = agent;
  formOpen.value = true;
}

// ============================================================================
// 删除
// ============================================================================

/** 请求删除（弹出确认） */
function requestDelete(agent: Agent): void {
  deleteTarget.value = agent;
  deleteDialogOpen.value = true;
}

/** 确认删除 */
async function confirmDelete(): Promise<void> {
  if (!deleteTarget.value) return;
  const id = deleteTarget.value.id;
  deleteDialogOpen.value = false;
  try {
    await agentsStore.deleteOne(id);
    toast.success("已删除");
  } catch {
    toast.error("删除失败");
  }
  deleteTarget.value = null;
}

/** 取消删除 */
function cancelDelete(): void {
  deleteDialogOpen.value = false;
  deleteTarget.value = null;
}

// ============================================================================
// 表单提交
// ============================================================================

/** 处理表单提交 */
async function handleFormSubmit(payload: AgentFormPayload): Promise<void> {
  try {
    if (formMode.value === "create") {
      await agentsStore.createOne({
        name: payload.name,
        provider: payload.provider,
        model: payload.model,
        api_key: payload.api_key,
        base_url: payload.base_url || undefined,
        system_prompt: payload.system_prompt || undefined,
        temperature: payload.temperature,
        max_tokens: payload.max_tokens,
      });
      toast.success("Agent 已创建");
    } else if (editingAgent.value) {
      // 先更新基本信息（id 由 updateOne 内部拼入 patch）
      await agentsStore.updateOne(editingAgent.value.id, {
        id: editingAgent.value.id,
        name: payload.name,
        provider: payload.provider,
        model: payload.model,
        base_url: payload.base_url || undefined,
        system_prompt: payload.system_prompt || undefined,
        temperature: payload.temperature,
        max_tokens: payload.max_tokens,
      });
      // 如果需要轮换 key
      if (payload.rotateApiKey && payload.api_key) {
        await agentsStore.rotateKey(
          editingAgent.value.id,
          payload.api_key,
          payload.base_url || undefined,
        );
      }
      toast.success("Agent 已更新");
    }
    formOpen.value = false;
  } catch {
    toast.error("保存失败");
  }
}
</script>

<template>
  <div class="agent-manager">
    <!-- 页头 -->
    <div class="page-header">
      <h1 class="page-title">Agent 管理</h1>
      <button class="btn-add" @click="openCreate">+ 新建 Agent</button>
    </div>

    <!-- 加载中 -->
    <div v-if="agentsStore.loading" class="loading-hint">
      <span>加载中...</span>
    </div>

    <!-- 列表 / 空状态 -->
    <template v-else-if="agentsStore.hasAgents">
      <div class="agent-list">
        <AgentCard
          v-for="agent in agentsStore.agents"
          :key="agent.id"
          :agent="agent"
          @edit="openEdit"
          @delete="requestDelete"
        />
      </div>
    </template>
    <template v-else>
      <EmptyAgentHint @create="openCreate" />
    </template>
  </div>

  <!-- 创建/编辑表单面板 -->
  <AgentForm
    :mode="formMode"
    :agent="editingAgent"
    :open="formOpen"
    @update:open="formOpen = $event"
    @submit="handleFormSubmit"
  />

  <!-- 删除确认弹窗 -->
  <ConfirmDeleteDialog
    :open="deleteDialogOpen"
    title="确认删除"
    :message="`将删除 Agent「${deleteTarget?.name ?? ''}」及其所有会话，此操作不可撤销。`"
    @confirm="confirmDelete"
    @cancel="cancelDelete"
  />
</template>

<style scoped>
.agent-manager {
  max-width: 720px;
  margin: 0 auto;
  padding: 32px 24px;
}

.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--ip-spacing-6);
}

.page-title {
  margin: 0;
  font-size: var(--ip-text-h2-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.btn-add {
  padding: var(--ip-btn-py-md) 18px;
  font-size: var(--ip-btn-fs-md);
  font-weight: var(--ip-font-weight-medium);
  border: none;
  border-radius: var(--ip-btn-radius);
  background: var(--ip-primary-500);
  color: var(--ip-color-text-on-primary);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-add:hover {
  background: var(--ip-primary-600);
}

.agent-list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
}

.loading-hint {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 48px;
  color: var(--ip-color-text-tertiary);
}
</style>