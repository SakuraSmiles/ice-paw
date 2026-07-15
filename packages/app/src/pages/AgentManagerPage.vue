<script setup lang="ts">
// Agent 管理页
//
// 职责：
//   - 顶部「+ 新建 Agent」按钮（@ice-paw/ui Button + lucide Plus 图标）
//   - Agent 卡片列表（v-for AgentCard）
//   - 无数据时显示 EmptyAgentHint
//   - 挂载 AgentForm（侧滑面板）+ ConfirmDeleteDialog（Modal）
//   - 处理创建/编辑/删除的全部业务逻辑

import { ref, onMounted } from "vue";
import { Button } from "@ice-paw/ui";
import { Plus } from "lucide-vue-next";
import { useAgentsStore } from "../stores/agents";
import { useToast } from "../composables/useToast";
import { useAgentMeta } from "../composables/useAgentMeta";
import { findTemplateById } from "../data/agentTemplates";
import { initialsFromName } from "../utils/agentAvatar";
import type { Agent } from "../types";
import type { AgentFormPayload } from "../components/agent/AgentForm.vue";
import AgentCard from "../components/agent/AgentCard.vue";
import AgentForm from "../components/agent/AgentForm.vue";
import ConfirmDeleteDialog from "../components/agent/ConfirmDeleteDialog.vue";
import EmptyAgentHint from "../components/agent/EmptyAgentHint.vue";

const agentsStore = useAgentsStore();
const toast = useToast();
const agentMeta = useAgentMeta();

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
    // 同步清理 agentMeta localStorage，避免遗留
    agentMeta.removeMeta(id);
    toast.success("已删除");
  } catch {
    toast.error("删除失败");
  }
  deleteTarget.value = null;
}

/** 删除弹窗关闭（取消 / Esc / 遮罩 / 关闭按钮） */
function onDeleteDialogClose(): void {
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
      const created = await agentsStore.createOne({
        name: payload.name,
        provider: payload.provider,
        model: payload.model,
        api_key: payload.api_key,
        base_url: payload.base_url || undefined,
        system_prompt: payload.system_prompt || undefined,
        temperature: payload.temperature,
        max_tokens: payload.max_tokens,
        cache_prompt: payload.cachePrompt,
        // A3-2: 历史窗口（undefined 表示不传该字段，Rust 侧落库为 NULL）
        max_history_messages: payload.maxHistoryMessages ?? undefined,
      });
      // 若用户选中了模板，写 meta 到 localStorage
      if (payload.templateId) {
        const tpl = findTemplateById(payload.templateId);
        if (tpl) {
          agentMeta.setMeta(created.id, {
            avatarText: initialsFromName(tpl.name),
            avatarColor: tpl.color,
            icon: tpl.icon,
            description: tpl.description,
            promptChips: tpl.promptChips,
          });
        }
      }
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
        cache_prompt: payload.cachePrompt,
        // A3-2: 历史窗口（undefined=不更新 / null=清空 / 数字=设值）
        max_history_messages: payload.maxHistoryMessages,
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
      <Button variant="primary" size="md" @click="openCreate">
        <template #icon-left>
          <Plus :size="16" aria-hidden="true" />
        </template>
        新建 Agent
      </Button>
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
    v-model:open="deleteDialogOpen"
    title="确认删除"
    :message="`将删除 Agent「${deleteTarget?.name ?? ''}」及其所有会话，此操作不可撤销。`"
    @confirm="confirmDelete"
    @update:open="onDeleteDialogClose"
  />
</template>

<style scoped>
.agent-manager {
  max-width: 720px;
  margin: 0 auto;
  padding: var(--ip-spacing-8) var(--ip-spacing-6);
}

.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-4);
  margin-bottom: var(--ip-spacing-6);
}

.page-title {
  margin: 0;
  font-size: var(--ip-text-h2-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
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
  padding: var(--ip-spacing-12);
  color: var(--ip-color-text-tertiary);
}
</style>