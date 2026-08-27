<script setup lang="ts">
// ConfigProposalCard.vue — 配置提案内联审批卡片
// 在聊天流中渲染（非 modal），用户查看提案字段并批准/拒绝/编辑。
import { ref, computed } from "vue";
import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import { bridge } from "../../api/bridge";
import type {
  ConfigProposalPayload,
  ConfigProposalResponse,
  ProposalActionCreateAgent,
  ProposalActionUpdateAgent,
} from "../../types";

const chat = useChatStore();

const props = defineProps<{
  proposal: ConfigProposalPayload;
}>();

const apiKey = ref("");
const editing = ref(false);
const applying = ref(false);
const errorMsg = ref<string | null>(null);

// 编辑模式下的可变字段副本
const editFields = ref<Record<string, string>>({});

const isCreate = computed(() => props.proposal.action.action === "create_agent");
const action = computed(() =>
  isCreate.value
    ? (props.proposal.action as ProposalActionCreateAgent)
    : (props.proposal.action as ProposalActionUpdateAgent),
);

const sensitivityLabel = computed(() => {
  switch (props.proposal.sensitivity) {
    case "low": return "🟢 非敏感";
    case "medium": return "🟡 敏感";
    default: return "🔴 红线";
  }
});

const sensitivityClass = computed(() => `sensitivity-${props.proposal.sensitivity}`);

/** 可显示的字段列表 */
const visibleFields = computed(() => {
  const a = action.value;
  const fields: { key: string; label: string; value: unknown; isKeySlot: boolean }[] = [];

  if (isCreate.value) {
    const c = a as ProposalActionCreateAgent;
    fields.push({ key: "id", label: "ID", value: c.id, isKeySlot: false });
    fields.push({ key: "name", label: "名称", value: c.name, isKeySlot: false });
    fields.push({ key: "provider", label: "提供商", value: c.provider, isKeySlot: false });
    fields.push({ key: "model", label: "模型", value: c.model, isKeySlot: false });
    if (c.system_prompt) fields.push({ key: "system_prompt", label: "系统提示", value: c.system_prompt, isKeySlot: false });
    if (c.temperature != null) fields.push({ key: "temperature", label: "Temperature", value: c.temperature, isKeySlot: false });
    if (c.max_tokens != null) fields.push({ key: "max_tokens", label: "Max Tokens", value: c.max_tokens, isKeySlot: false });
    if (c.base_url) fields.push({ key: "base_url", label: "Base URL", value: c.base_url, isKeySlot: false });
    if (c.enabled_tools?.length) fields.push({ key: "enabled_tools", label: "启用工具", value: c.enabled_tools.join(", "), isKeySlot: false });
    if (c.workspace_path) fields.push({ key: "workspace_path", label: "工作区", value: c.workspace_path, isKeySlot: false });
    // API key: always shown as secure input
    fields.push({ key: "api_key", label: "API Key", value: "（必填）", isKeySlot: true });
  } else {
    const u = a as ProposalActionUpdateAgent;
    fields.push({ key: "agent_id", label: "Agent ID", value: u.agent_id, isKeySlot: false });
    if (u.name != null) fields.push({ key: "name", label: "名称", value: u.name, isKeySlot: false });
    if (u.provider != null) fields.push({ key: "provider", label: "提供商", value: u.provider, isKeySlot: false });
    if (u.model != null) fields.push({ key: "model", label: "模型", value: u.model, isKeySlot: false });
    if (u.system_prompt != null) fields.push({ key: "system_prompt", label: "系统提示", value: u.system_prompt, isKeySlot: false });
    if (u.temperature != null) fields.push({ key: "temperature", label: "Temperature", value: u.temperature, isKeySlot: false });
    if (u.max_tokens != null) fields.push({ key: "max_tokens", label: "Max Tokens", value: u.max_tokens, isKeySlot: false });
    if (u.base_url != null) fields.push({ key: "base_url", label: "Base URL", value: u.base_url, isKeySlot: false });
    if (u.enabled_tools != null) fields.push({ key: "enabled_tools", label: "启用工具", value: u.enabled_tools.join(", "), isKeySlot: false });
    if (u.workspace_path != null) fields.push({ key: "workspace_path", label: "工作区", value: u.workspace_path, isKeySlot: false });
    // Word 样式偏好：空串 = 摘除（卡片上明示，避免误以为写成空块）
    if (u.word_style_profile != null) {
      fields.push({
        key: "word_style_profile",
        label: "Word 样式偏好",
        value: u.word_style_profile === "" ? "（摘除现有偏好）" : u.word_style_profile,
        isKeySlot: false,
      });
    }
  }

  return fields;
});

function fieldDisplayValue(field: { key: string; value: unknown }): string {
  if (editing.value && editFields.value[field.key] !== undefined) {
    return editFields.value[field.key];
  }
  if (field.value === null || field.value === undefined) return "—";
  return String(field.value);
}

function startEdit() {
  editing.value = true;
  const fields: Record<string, string> = {};
  for (const f of visibleFields.value) {
    // 多行自由文字块不入单行编辑（String() 会压掉换行，损坏原文）
    if (!f.isKeySlot && f.key !== "word_style_profile") {
      fields[f.key] = String(f.value ?? "");
    }
  }
  editFields.value = fields;
}

function cancelEdit() {
  editing.value = false;
  editFields.value = {};
}

async function approve() {
  errorMsg.value = null;
  applying.value = true;

  try {
    if (isCreate.value) {
      const a = action.value as ProposalActionCreateAgent;
      const key = apiKey.value.trim();
      if (!key) {
        errorMsg.value = "请填写 API Key";
        applying.value = false;
        return;
      }
      await bridge.agents.create({
        id: a.id,
        name: editing.value ? (editFields.value.name || a.name) : a.name,
        provider: editing.value ? (editFields.value.provider || a.provider) : a.provider,
        model: editing.value ? (editFields.value.model || a.model) : a.model,
        api_key: key,
        base_url: a.base_url ?? undefined,
        system_prompt: a.system_prompt ?? undefined,
        temperature: a.temperature ?? undefined,
        max_tokens: a.max_tokens ?? undefined,
        enabled_tools: a.enabled_tools ?? undefined,
        workspace_path: a.workspace_path ?? undefined,
      });
    } else {
      const a = action.value as ProposalActionUpdateAgent;
      // 编辑值覆盖优先：批准的东西 = 卡片上现在显示的值（创建路径同款形状）
      const edited = (key: string): string | undefined =>
        editing.value && editFields.value[key] !== undefined ? editFields.value[key] : undefined;
      const val = (key: string, orig: string | null | undefined): string | undefined => {
        const e = edited(key);
        if (e !== undefined) return e.trim() === "" ? undefined : e.trim();
        return orig ?? undefined;
      };
      // ① 出生证字段 → update_agent（DB 列）
      await bridge.agents.update({
        id: a.agent_id,
        name: val("name", a.name),
        provider: val("provider", a.provider),
        model: val("model", a.model),
        base_url: val("base_url", a.base_url),
        workspace_path: val("workspace_path", a.workspace_path),
      });
      // ② 旋钮字段 → yaml 通道（出生 yaml 的活行遮蔽 DB 列，写 DB 是「假生效」；
      //    旋钮唯一权威 = agent.yaml，update_agent 只管出生证）
      if (a.system_prompt != null || edited("system_prompt")) {
        const sp = edited("system_prompt")?.trim() || a.system_prompt;
        if (sp) await bridge.agents.setSystemPrompt(a.agent_id, sp);
      }
      const applyNum = (key: "temperature" | "max_tokens", orig: number | null | undefined) => {
        if (orig == null && edited(key) === undefined) return Promise.resolve();
        const e = edited(key);
        const v = e === undefined ? orig : e.trim() === "" ? null : Number(e);
        if (v === undefined) return Promise.resolve();
        if (typeof v === "number" && Number.isNaN(v)) {
          return Promise.reject(new Error(`${key} 不是合法数字`));
        }
        return bridge.agents.setYamlField(a.agent_id, key, v);
      };
      await applyNum("temperature", a.temperature);
      await applyNum("max_tokens", a.max_tokens);
      if (a.enabled_tools != null || edited("enabled_tools")) {
        const e = edited("enabled_tools");
        const list = e !== undefined
          ? e.split(",").map((s) => s.trim()).filter(Boolean)
          : (a.enabled_tools ?? []);
        await bridge.agents.setEnabledTools(a.agent_id, list);
      }
      // Word 样式偏好走 agent.yaml 专用命令（纯文件旁路，不进 DB 字段）；
      // `""` = 摘除语义由后端命令解释
      if (a.word_style_profile !== undefined) {
        await bridge.agents.setWordProfile(a.agent_id, a.word_style_profile ?? null);
      }
    }

    // 刷新 agent store 缓存
    useAgentStore().load(true);

    // 通知 Rust 端继续对话
    applying.value = false;
    const response: ConfigProposalResponse = {
      request_id: props.proposal.request_id,
      decision: editing.value ? "modified" : "approved",
    };
    if (editing.value) {
      response.changes = editFields.value;
    }
    await chat.respondToProposal(response);
  } catch (e) {
    errorMsg.value = `应用失败: ${e instanceof Error ? e.message : String(e)}`;
    applying.value = false;
  }
}

async function reject() {
  await chat.respondToProposal({
    request_id: props.proposal.request_id,
    decision: "rejected",
    reason: "用户拒绝",
  });
}
</script>

<template>
  <div :class="['proposal-card', sensitivityClass]">
    <!-- 头部 -->
    <div class="proposal-header">
      <span class="proposal-badge">{{ sensitivityLabel }}</span>
      <span class="proposal-title">{{ isCreate ? '创建 Agent' : '更新 Agent' }}</span>
    </div>

    <!-- 摘要 -->
    <div class="proposal-summary">{{ proposal.summary }}</div>

    <!-- 字段列表 -->
    <div class="proposal-fields">
      <div
        v-for="field in visibleFields"
        :key="field.key"
        class="proposal-field"
      >
        <span class="field-label">{{ field.label }}</span>
        <span v-if="!field.isKeySlot" class="field-value">{{ fieldDisplayValue(field) }}</span>
        <!-- API Key 安全输入 -->
        <input
          v-if="field.isKeySlot"
          v-model="apiKey"
          type="password"
          class="field-input"
          placeholder="输入 API Key（必填）"
          autocomplete="off"
        />
        <!-- 编辑模式下的可编辑字段（agent_id/id 只读；多行偏好块不入单行编辑） -->
        <input
          v-if="editing && !field.isKeySlot && field.key !== 'agent_id' && field.key !== 'id' && field.key !== 'word_style_profile'"
          v-model="editFields[field.key]"
          type="text"
          class="field-input"
        />
      </div>
    </div>

    <!-- 错误提示 -->
    <div v-if="errorMsg" class="proposal-error">{{ errorMsg }}</div>

    <!-- 操作按钮 -->
    <div class="proposal-footer">
      <button class="btn btn-reject" :disabled="applying" @click="reject">拒绝</button>
      <button v-if="!editing" class="btn btn-edit" :disabled="applying" @click="startEdit">编辑</button>
      <button v-if="editing" class="btn btn-cancel" @click="cancelEdit">取消编辑</button>
      <button class="btn btn-approve" :disabled="applying" @click="approve">
        {{ applying ? '应用中…' : (editing ? '应用修改' : '批准') }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.proposal-card {
  margin: 8px 0;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  background: var(--ip-color-bg-elevated);
  overflow: hidden;
}

.proposal-card.sensitivity-low {
  border-left: 3px solid var(--ip-success-base);
}

.proposal-card.sensitivity-medium {
  border-left: 3px solid var(--ip-warning-base);
}

.proposal-card.sensitivity-redline {
  border-left: 3px solid var(--ip-danger-base);
}

.proposal-header {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: 10px 14px 4px;
}

.proposal-badge {
  font-size: var(--ip-text-micro-size);
  font-weight: var(--ip-font-weight-semibold);
  padding: 1px 8px;
  border-radius: var(--ip-radius-full);
}

.sensitivity-low .proposal-badge {
  background: var(--ip-success-soft-bg);
  color: var(--ip-success-tint-text);
}

.sensitivity-medium .proposal-badge {
  background: var(--ip-warning-soft-bg);
  color: var(--ip-warning-tint-text);
}

.sensitivity-redline .proposal-badge {
  background: var(--ip-danger-soft-bg);
  color: var(--ip-danger-tint-text);
}

.proposal-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.proposal-summary {
  padding: 2px 14px 8px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  line-height: 1.5;
}

.proposal-fields {
  padding: 0 14px 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.proposal-field {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
}

.field-label {
  color: var(--ip-color-text-tertiary);
  min-width: 80px;
  flex-shrink: 0;
  text-align: right;
}

.field-value {
  color: var(--ip-color-text-primary);
  font-family: var(--ip-font-mono, monospace);
  font-size: var(--ip-text-micro-size);
  word-break: break-all;
}

.field-input {
  flex: 1;
  padding: 2px 6px;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  font-size: var(--ip-text-micro-size);
  font-family: var(--ip-font-mono, monospace);
  background: var(--ip-color-bg-secondary);
  color: var(--ip-color-text-primary);
}

.field-input:focus {
  outline: none;
  border-color: var(--ip-primary-500);
}

.proposal-error {
  margin: 0 14px 8px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-danger-base);
  padding: 4px 8px;
  background: var(--ip-danger-soft-bg);
  border-radius: var(--ip-radius-sm);
}

.proposal-footer {
  display: flex;
  gap: 6px;
  padding: 6px 14px 10px;
  border-top: 1px solid var(--ip-color-border-default);
}

.btn {
  padding: 4px 12px;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  cursor: pointer;
  border: none;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-reject {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
}
.btn-reject:hover:not(:disabled) {
  background: var(--ip-danger-soft-bg);
  color: var(--ip-danger-base);
}

.btn-edit {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
}
.btn-edit:hover:not(:disabled) {
  background: var(--ip-color-bg-secondary);
}

.btn-cancel {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
}

.btn-approve {
  background: var(--ip-primary-500);
  color: white;
  margin-left: auto;
}
.btn-approve:hover:not(:disabled) {
  opacity: 0.9;
}
</style>
