<script setup lang="ts">
// AgentSettings.vue — 智能体设置
import { ref, onMounted } from "vue";
import AgentFormModal from "../../components/agent/AgentFormModal.vue";
import KbDocumentList from "../../components/kb/KbDocumentList.vue";
import type { Agent } from "../../types";
import { bridge } from "../../api/bridge";

const agents = ref<Agent[]>([]);
const loading = ref(true);

const showForm = ref(false);
const editingAgent = ref<Agent | null>(null);

// 知识库展开状态（卡片内展开，仿 McpSettings tools-toggle）
const expandedKbId = ref<string | null>(null);
function toggleKb(agentId: string) {
  expandedKbId.value = expandedKbId.value === agentId ? null : agentId;
}

async function loadAgents() {
  loading.value = true;
  try {
    agents.value = await bridge.agents.list();
  } catch (e) {
    console.error("加载 Agent 列表失败:", e);
  } finally {
    loading.value = false;
  }
}

onMounted(loadAgents);

function openNew() { editingAgent.value = null; showForm.value = true; }
function openEdit(agent: Agent) { editingAgent.value = agent; showForm.value = true; }
function closeForm() { showForm.value = false; editingAgent.value = null; }

async function onSaved(_agent: Agent) {
  await loadAgents();
  closeForm();
}

async function onDelete(agent: Agent) {
  try {
    await bridge.agents.delete(agent.id);
    await loadAgents();
    closeForm();
  } catch (e) {
    console.error("删除 Agent 失败:", e);
  }
}

const providerLabels: Record<string, string> = {
  openai: "OpenAI", anthropic: "Anthropic", deepseek: "DeepSeek",
  glm: "GLM", minimax: "MiniMax", "minimax-cn": "MiniMax(CN)",
};
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">智能体</h2>
      <button class="btn-primary" @click="openNew">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        新建
      </button>
    </div>

    <div class="agent-list">
      <div v-for="agent in agents" :key="agent.id" class="agent-card" @click="openEdit(agent)">
        <div class="card-top">
          <div class="card-avatar">{{ agent.name.charAt(0) }}</div>
          <div class="card-body">
            <div class="card-name-row">
              <span class="card-name">{{ agent.name }}</span>
              <span v-if="agent.config_from_file" class="card-file-badge">agent.yaml</span>
            </div>
            <div class="card-meta-row">
              <span class="provider-badge" :class="'provider-' + agent.provider">{{ providerLabels[agent.provider] || agent.provider }}</span>
              <span class="card-model">{{ agent.model }}</span>
              <span v-if="agent.supports_vision" class="card-tag">Vision</span>
              <span v-if="agent.cache_prompt" class="card-tag">Cache</span>
              <span v-if="!agent.has_api_key" class="card-tag card-tag-warn">未配置 Key</span>
            </div>
          </div>
          <svg class="card-chevron" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </div>
        <div v-if="agent.description" class="card-desc">{{ agent.description }}</div>
        <div v-if="agent.workspace_path" class="card-workspace">{{ agent.workspace_path }}</div>

        <div class="kb-toggle" @click.stop="toggleKb(agent.id)">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" :class="{ rotated: expandedKbId === agent.id }">
            <polyline points="6 9 12 15 18 9" />
          </svg>
          {{ expandedKbId === agent.id ? "收起知识库" : "查看知识库" }}
        </div>
        <div v-if="expandedKbId === agent.id" class="kb-panel" @click.stop>
          <KbDocumentList scope="agent" :owner-id="agent.id" />
        </div>
      </div>

      <div v-if="loading" class="loading-state">加载中...</div>
      <div v-else-if="agents.length === 0" class="empty-state">
        <div class="empty-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
            <path d="M7 11V7a5 5 0 0 1 10 0v4" />
          </svg>
        </div>
        <h3 class="empty-title">还没有智能体</h3>
        <p class="empty-desc">创建你的第一个 AI 助手，开始对话</p>
        <button class="btn-primary" @click="openNew">新建 Agent</button>
      </div>
    </div>

    <AgentFormModal v-if="showForm" :agent="editingAgent" @close="closeForm" @saved="onSaved" @delete="onDelete" />
  </div>
</template>

<style scoped>
.settings-content-inner { flex: 1; display: flex; flex-direction: column; padding: 0; min-height: 0; }

.content-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 20px 28px 0; flex-shrink: 0; height: 56px;
}
.content-title {
  font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin: 0;
}

.btn-primary {
  display: flex; align-items: center; gap: 6px; padding: 0 14px; height: 32px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  color: white; background-color: var(--ip-primary-600); border: none;
  border-radius: var(--ip-radius-md); cursor: pointer; white-space: nowrap;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary:hover { background-color: var(--ip-primary-700); }

/* ===== 列表 ===== */
.agent-list {
  flex: 1; overflow-y: auto; padding: 8px 28px 24px;
  display: flex; flex-direction: column; gap: 8px; min-height: 0;
}

/* ===== 卡片 ===== */
.agent-card {
  padding: 14px 16px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.agent-card:hover {
  border-color: var(--ip-primary-300);
  box-shadow: var(--ip-shadow-sm);
}

.card-top {
  display: flex;
  align-items: center;
  gap: 12px;
}

.card-avatar {
  width: 36px; height: 36px; border-radius: var(--ip-radius-md);
  background: linear-gradient(135deg, var(--ip-primary-400), var(--ip-primary-600));
  color: white; display: flex; align-items: center; justify-content: center;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
  flex-shrink: 0;
}

.card-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.card-name-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.card-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.card-file-badge {
  display: inline-flex; align-items: center;
  height: 18px; padding: 0 6px;
  font-size: 9px; font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-primary-700);
  background-color: var(--ip-primary-100);
  border-radius: var(--ip-radius-full);
  font-family: var(--ip-font-mono);
}

.card-meta-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
}

.provider-badge {
  display: inline-block; padding: 0 6px; line-height: 18px;
  font-size: 10px; font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-full);
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
  flex-shrink: 0;
}
.provider-badge.provider-openai { background: #E8F4EE; color: #237552; }
.provider-badge.provider-anthropic { background: #EDE8F4; color: #5B3D8A; }
.provider-badge.provider-deepseek { background: #F4E8E8; color: #8A3D3D; }
.provider-badge.provider-glm { background: #E8EEF4; color: #3D5B8A; }

.card-model {
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

.card-tag {
  flex-shrink: 0;
  font-size: 10px; color: var(--ip-color-text-tertiary);
  background: var(--ip-color-bg-tertiary);
  padding: 0 6px; line-height: 18px;
  border-radius: var(--ip-radius-full);
}
.card-tag-warn { color: var(--ip-warning-text); background: var(--ip-warning-bg); }

.card-chevron {
  flex-shrink: 0;
  color: var(--ip-color-text-disabled);
}

.card-desc {
  margin: 8px 0 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}

.card-workspace {
  margin: 4px 0 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-disabled);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  padding-left: 48px;
  font-family: var(--ip-font-mono);
}

/* ===== 状态 ===== */
.loading-state { flex: 1; display: flex; align-items: center; justify-content: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }
.empty-state { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 40px; }
.empty-icon { width: 48px; height: 48px; margin-bottom: 8px; color: var(--ip-color-text-tertiary); }
.empty-title { font-size: var(--ip-text-body-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); margin: 0; }
.empty-desc { font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); margin: 0 0 8px; }

/* ===== 知识库展开（卡片内，仿 McpSettings tools-toggle） ===== */
.kb-toggle {
  display: flex;
  align-items: center;
  gap: 5px;
  margin-top: 8px;
  padding-left: 48px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  user-select: none;
}
.kb-toggle:hover { color: var(--ip-primary-600); }
.kb-toggle svg { transition: transform var(--ip-duration-fast) var(--ip-ease-out); }
.kb-toggle svg.rotated { transform: rotate(180deg); }

.kb-panel {
  margin-top: 8px;
  padding: 12px;
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-md);
}
</style>
