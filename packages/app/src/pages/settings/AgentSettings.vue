<script setup lang="ts">
// AgentSettings.vue — 智能体设置
import { ref, onMounted } from "vue";
import AgentFormModal from "../../components/agent/AgentFormModal.vue";
import type { Agent } from "../../types";
import { bridge } from "../../api/bridge";

const agents = ref<Agent[]>([]);
const loading = ref(true);

const showForm = ref(false);
const editingAgent = ref<Agent | null>(null);

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
  // 刷新列表拿到最新数据（包含后端生成的 id/时间戳等）
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
  openai: "OpenAI", anthropic: "Anthropic", deepseek: "DeepSeek", glm: "GLM", minimax: "MiniMax", "minimax-cn": "MiniMax(CN)",
};
</script>

<template>
  <div class="settings-content-inner">
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
            <div class="card-id">{{ agent.id }}</div>
            <div class="card-model">
              <span class="provider-badge" :class="'provider-' + agent.provider">{{ providerLabels[agent.provider] || agent.provider }}</span>
              {{ agent.model }}
            </div>
          </div>
          <div class="card-status">
            <span v-if="agent.config_from_file" class="status-badge status-file" title="部分配置来自 agent.yaml">File</span>
            <span v-if="agent.has_api_key" class="status-badge status-ok">已配置</span>
            <span v-else class="status-badge status-warn">未配置 Key</span>
          </div>
        </div>
        <p v-if="agent.description" class="card-desc">{{ agent.description }}</p>
        <div v-if="agent.workspace_path" class="card-workspace">{{ agent.workspace_path }}</div>
        <div class="card-meta">
          <span v-if="agent.supports_vision" class="meta-tag">Vision</span>
          <span v-if="agent.cache_prompt" class="meta-tag">Cache</span>
          <span class="meta-time">更新于 {{ agent.updated_at }}</span>
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
        <h2 class="empty-title">还没有智能体</h2>
        <p class="empty-desc">创建你的第一个 AI 助手，开始对话</p>
        <button class="btn-primary" @click="openNew">新建 Agent</button>
      </div>
    </div>

    <AgentFormModal v-if="showForm" :agent="editingAgent" @close="closeForm" @saved="onSaved" @delete="onDelete" />
  </div>
</template>

<style scoped>
.settings-content-inner { height: 100%; display: flex; flex-direction: column; padding: 24px; }

.content-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 0 0 20px;
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
  padding: 16px 20px; background-color: var(--ip-color-bg-secondary);
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
.card-id { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); font-family: var(--ip-font-mono); }
.card-model { font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); display: flex; align-items: center; gap: 6px; }
.provider-badge { display: inline-block; padding: 1px 7px; font-size: 11px; font-weight: var(--ip-font-weight-medium); border-radius: var(--ip-radius-full); background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-secondary); }
.provider-badge.provider-openai { background: #E8F4EE; color: #237552; }
.provider-badge.provider-anthropic { background: #EDE8F4; color: #5B3D8A; }
.provider-badge.provider-deepseek { background: #F4E8E8; color: #8A3D3D; }
.provider-badge.provider-glm { background: #E8EEF4; color: #3D5B8A; }
.card-status { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
.status-badge { display: inline-block; padding: 2px 10px; font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium); border-radius: var(--ip-radius-full); }
.status-ok { background-color: var(--ip-success-bg); color: var(--ip-success-text); }
.status-warn { background-color: var(--ip-warning-bg); color: var(--ip-warning-text); }
.status-file { background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); font-size: 10px; }
.card-desc { margin: 10px 0 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-tertiary); line-height: 1.5; }
.card-workspace { margin: 6px 0 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.card-meta { display: flex; align-items: center; gap: 8px; margin-top: 10px; }
.meta-tag { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.meta-time { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); margin-left: auto; }
.loading-state { flex: 1; display: flex; align-items: center; justify-content: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }
.empty-state { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 40px; }
.empty-icon { width: 48px; height: 48px; margin-bottom: 8px; color: var(--ip-color-text-tertiary); }
.empty-title { font-size: var(--ip-text-h2-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); margin: 0; }
.empty-desc { font-size: var(--ip-text-body-size); color: var(--ip-color-text-secondary); margin: 0 0 8px; }
</style>
