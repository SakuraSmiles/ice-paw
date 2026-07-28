<script setup lang="ts">
// AgentSettings.vue — 智能体设置
import { ref } from "vue";
import AgentFormModal from "../../components/agent/AgentFormModal.vue";
import type { Agent } from "../../types";

const agents = ref<Agent[]>([
  {
    id: "1", name: "代码助手", provider: "openai", model: "gpt-4o",
    system_prompt: "你是专业的高级软件工程师，擅长代码审查、架构设计和性能优化。分析问题时请从多个角度考虑（正确性、可维护性、性能、安全性），并给出具体的代码示例。",
    base_url: null, temperature: 0.3, max_tokens: 4096, extra_params: {}, sort_order: 0,
    cache_prompt: true, max_history_messages: 40, supports_vision: true,
    description: "专业的代码审查与架构设计助手，适用于开发和代码分析场景",
    has_api_key: true, created_at: "2026-07-20T10:00:00Z", updated_at: "2026-07-28T10:00:00Z",
  },
  {
    id: "2", name: "写作助手", provider: "anthropic", model: "claude-sonnet-4-20250514",
    system_prompt: "你是出色的中文写作专家，擅长润色、改写和创意写作。保持原文风格的同时提升表达质量。",
    base_url: null, temperature: 0.7, max_tokens: 8192, extra_params: {}, sort_order: 1,
    cache_prompt: true, description: "中文写作润色与创意文案助手",
    has_api_key: false, created_at: "2026-07-25T14:00:00Z", updated_at: "2026-07-28T10:00:00Z",
  },
  {
    id: "3", name: "通用问答", provider: "deepseek", model: "deepseek-chat",
    system_prompt: "", base_url: null, temperature: 0.7, max_tokens: 4096,
    extra_params: {}, sort_order: 2, cache_prompt: false,
    description: "通用知识问答助手", has_api_key: true,
    created_at: "2026-07-22T09:00:00Z", updated_at: "2026-07-27T16:00:00Z",
  },
  {
    id: "4", name: "翻译助手", provider: "glm", model: "glm-4-flash",
    system_prompt: "你是一名专业的翻译专家，精通中英互译。保持原文语气和风格，准确传达语义。",
    base_url: null, temperature: 0.3, max_tokens: 4096,
    extra_params: {}, sort_order: 3, cache_prompt: true,
    description: "中英翻译与本地化助手", has_api_key: true,
    created_at: "2026-07-26T11:00:00Z", updated_at: "2026-07-28T09:00:00Z",
  },
  {
    id: "5", name: "数据分析师", provider: "openai", model: "o3-mini",
    system_prompt: "你是资深数据分析师，擅长数据清洗、统计分析和可视化方案设计。给出结论时附带置信度说明。",
    base_url: null, temperature: 0.2, max_tokens: 8192,
    extra_params: {}, sort_order: 4, cache_prompt: false,
    description: "数据清洗、统计分析与可视化方案设计", has_api_key: true,
    created_at: "2026-07-24T15:00:00Z", updated_at: "2026-07-28T10:00:00Z",
  },
]);

const showForm = ref(false);
const editingAgent = ref<Agent | null>(null);

function openNew() { editingAgent.value = null; showForm.value = true; }
function openEdit(agent: Agent) { editingAgent.value = agent; showForm.value = true; }
function closeForm() { showForm.value = false; editingAgent.value = null; }

function onSaved(agent: Agent) {
  if (editingAgent.value) {
    const idx = agents.value.findIndex((a) => a.id === agent.id);
    if (idx >= 0) agents.value[idx] = agent;
  } else agents.value.unshift(agent);
  closeForm();
}
function onDelete(agent: Agent) { agents.value = agents.value.filter((a) => a.id !== agent.id); closeForm(); }

const providerLabels: Record<string, string> = {
  openai: "OpenAI", anthropic: "Anthropic", deepseek: "DeepSeek", glm: "GLM", minimax: "MiniMax", "minimax-cn": "MiniMax(CN)",
};
</script>

<template>
  <div class="agent-content">
    <!-- 标题行 + 新建按钮 -->
    <div class="content-header">
      <h2 class="content-title">智能体</h2>
      <button class="btn-primary" @click="openNew">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" /></svg>
        新建 Agent
      </button>
    </div>

    <div class="agent-list">
      <div v-for="agent in agents" :key="agent.id" class="agent-card" @click="openEdit(agent)">
        <div class="card-header">
          <div class="card-avatar">{{ agent.name.charAt(0) }}</div>
          <div class="card-info">
            <div class="card-name">{{ agent.name }}</div>
            <div class="card-model">
              <span class="provider-badge" :class="'provider-' + agent.provider">{{ providerLabels[agent.provider] || agent.provider }}</span>
              {{ agent.model }}
            </div>
          </div>
          <div class="card-status">
            <span v-if="agent.has_api_key" class="status-badge status-ok">已配置</span>
            <span v-else class="status-badge status-warn">未配置 Key</span>
          </div>
        </div>
        <p v-if="agent.description" class="card-desc">{{ agent.description }}</p>
        <div class="card-meta">
          <span v-if="agent.supports_vision" class="meta-tag">Vision</span>
          <span v-if="agent.cache_prompt" class="meta-tag">Cache</span>
          <span class="meta-time">更新于 {{ agent.updated_at }}</span>
        </div>
      </div>

      <div v-if="agents.length === 0" class="empty-state">
        <div class="empty-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
            <path d="M7 11V7a5 5 0 0 1 10 0v4" />
          </svg>
        </div>
        <h2 class="empty-title">还没有智能体</h2>
        <p class="empty-desc">创建你的第一个 AI 助手，开始对话</p>
        <button class="btn-primary" @click="openNew">新建 Agent</button>
      </div>
    </div>

    <AgentFormModal v-if="showForm" :agent="editingAgent" @close="closeForm" @saved="onSaved" @delete="onDelete" />
  </div>
</template>

<style scoped>
.agent-content { flex: 1; display: flex; flex-direction: column; min-height: 0; }

.content-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 0 0 16px;
  flex-shrink: 0;
}
.content-title {
  font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin: 0;
}

.btn-primary {
  display: flex; align-items: center; gap: 6px; padding: 8px 16px; height: 36px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  color: white; background-color: var(--ip-primary-600); border: none;
  border-radius: var(--ip-radius-md); cursor: pointer; white-space: nowrap;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary:hover { background-color: var(--ip-primary-700); }

.agent-list { flex: 1; overflow-y: auto; padding: 0; display: flex; flex-direction: column; gap: 10px; min-height: 0; }
.agent-card {
  padding: 14px 18px; background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-xl);
  cursor: pointer; transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.agent-card:hover { border-color: var(--ip-primary-300); box-shadow: var(--ip-shadow-sm); }
.card-header { display: flex; align-items: center; gap: 14px; }
.card-avatar {
  width: 40px; height: 40px; border-radius: var(--ip-radius-lg);
  background: linear-gradient(135deg, var(--ip-primary-400), var(--ip-primary-600));
  color: white; display: flex; align-items: center; justify-content: center;
  font-size: var(--ip-text-body-size); font-weight: var(--ip-font-weight-semibold); flex-shrink: 0;
}
.card-info { flex: 1; display: flex; flex-direction: column; gap: 3px; min-width: 0; }
.card-name { font-size: var(--ip-text-body-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.card-model { font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); display: flex; align-items: center; gap: 6px; }
.provider-badge { display: inline-block; padding: 1px 7px; font-size: 11px; font-weight: var(--ip-font-weight-medium); border-radius: var(--ip-radius-full); background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-secondary); }
.provider-badge.provider-openai { background: #E8F4EE; color: #237552; }
.provider-badge.provider-anthropic { background: #EDE8F4; color: #5B3D8A; }
.provider-badge.provider-deepseek { background: #F4E8E8; color: #8A3D3D; }
.provider-badge.provider-glm { background: #E8EEF4; color: #3D5B8A; }
.card-status { flex-shrink: 0; }
.status-badge { display: inline-block; padding: 2px 10px; font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium); border-radius: var(--ip-radius-full); }
.status-ok { background-color: var(--ip-success-bg); color: var(--ip-success-text); }
.status-warn { background-color: var(--ip-warning-bg); color: var(--ip-warning-text); }
.card-desc { margin: 10px 0 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-tertiary); line-height: 1.5; }
.card-meta { display: flex; align-items: center; gap: 8px; margin-top: 10px; }
.meta-tag { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.meta-time { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); margin-left: auto; }
.empty-state { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 40px; }
.empty-icon { width: 48px; height: 48px; margin-bottom: 8px; color: var(--ip-color-text-tertiary); }
.empty-title { font-size: var(--ip-text-h2-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); margin: 0; }
.empty-desc { font-size: var(--ip-text-body-size); color: var(--ip-color-text-secondary); margin: 0 0 8px; }
</style>
