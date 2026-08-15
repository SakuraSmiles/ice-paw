<script setup lang="ts">
// AgentSettings.vue — 智能体设置（卡片展开内联编辑 + 顶部特殊新建卡片）
import { ref, computed, onMounted } from "vue";
import AgentForm from "../../components/agent/AgentForm.vue";
import KbDocumentList from "../../components/kb/KbDocumentList.vue";
import type { Agent, ProviderInfo } from "../../types";
import { bridge } from "../../api/bridge";
import { loadProviders, providerLabelOf } from "../../composables/useProviders";
import { useAgentStore } from "../../stores/agent";

const store = useAgentStore();
// 单一数据源：直接从 Pinia store 派生，避免本地 ref 与 store 不一致
const agents = computed<Agent[]>(() => store.list);
const loading = ref(true);

// 展开编辑的 agent id（null = 全部收起）；isCreating = 新建卡片展开态
const expandedEditId = ref<string | null>(null);
const isCreating = ref(false);

async function loadAgents() {
  loading.value = true;
  try {
    await store.load(true);
  } catch (e) {
    console.error("加载 Agent 列表失败:", e);
  } finally {
    loading.value = false;
  }
}

onMounted(loadAgents);
onMounted(async () => { providerList.value = await loadProviders(); });

function toggleEdit(agent: Agent) {
  isCreating.value = false; // 编辑时收起新建
  expandedEditId.value = expandedEditId.value === agent.id ? null : agent.id;
}

/** 新建卡片：点击展开/合上，合上即取消（仿侧边栏新建对话的入口模式） */
function toggleNew() {
  expandedEditId.value = null;
  isCreating.value = !isCreating.value;
}

function onSaved(_agent: Agent) {
  isCreating.value = false;
  expandedEditId.value = null;
  loadAgents(); // loadAgents 内部调用 store.load(true)，同步侧栏/项目选择器
}

function onCancel() {
  isCreating.value = false;
  expandedEditId.value = null;
}

async function onDelete(agent: Agent) {
  try {
    await bridge.agents.delete(agent.id);
    isCreating.value = false;
    expandedEditId.value = null;
    await loadAgents();
  } catch (e) {
    console.error("删除 Agent 失败:", e);
  }
}

// Provider 显示名走目录（单一真相源；未收录名回退原文）。与 AgentForm 共享缓存。
const providerList = ref<ProviderInfo[]>([]);
const providerLabel = (name: string) => providerLabelOf(providerList.value, name);
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">智能体</h2>
    </div>

    <div class="agent-list">
      <!-- 新建智能体（列表第一条特殊卡片，虚线边框，仿侧边栏「新建对话」） -->
      <div class="agent-card new-card" :class="{ expanded: isCreating }" @click="toggleNew">
        <div class="card-top">
          <div class="card-body">
            <div class="card-name-row">
              <svg class="new-plus" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
              </svg>
              <span class="card-name new-name">新建智能体</span>
            </div>
            <div class="card-meta-row">
              <span class="new-hint">创建一个新的智能体…</span>
            </div>
          </div>
          <svg class="card-chevron" :class="{ rotated: isCreating }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </div>
        <div v-if="isCreating" class="expand-panel" @click.stop>
          <AgentForm :agent="null" @saved="onSaved" @cancel="onCancel" @delete="onDelete" />
        </div>
      </div>

      <!-- 分隔线 -->
      <div class="list-divider"></div>

      <!-- agent 列表 -->
      <div
        v-for="agent in agents"
        :key="agent.id"
        class="agent-card"
        :class="{ expanded: expandedEditId === agent.id }"
        @click="toggleEdit(agent)"
      >
        <div class="card-top">
          <div class="card-avatar">{{ agent.name.charAt(0) }}</div>
          <div class="card-body">
            <div class="card-name-row">
              <span class="card-name">{{ agent.name }}</span>
              <span v-if="agent.config_from_file" class="card-file-badge">agent.yaml</span>
            </div>
            <div class="card-meta-row">
              <span class="provider-badge" :class="'provider-' + agent.provider">{{ providerLabel(agent.provider) }}</span>
              <span class="card-model">{{ agent.model }}</span>
              <span v-if="!agent.has_api_key" class="card-tag card-tag-warn">未配置 Key</span>
            </div>
          </div>
          <svg class="card-chevron" :class="{ rotated: expandedEditId === agent.id }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </div>

        <!-- 展开态：编辑表单 + 知识库 -->
        <div v-if="expandedEditId === agent.id" class="expand-panel" @click.stop>
          <AgentForm :agent="agent" @saved="onSaved" @cancel="onCancel" @delete="onDelete" />

          <!-- 知识库（caption 区段，无嵌套框） -->
          <div class="region">
            <span class="region-title">知识库</span>
            <KbDocumentList scope="agent" :owner-id="agent.id" flat />
          </div>
        </div>
      </div>

      <div v-if="loading && !agents.length" class="loading-state">加载中...</div>
      <div v-else-if="agents.length === 0" class="empty-hint">还没有其他智能体，点上方「新建智能体」创建</div>
    </div>
  </div>
</template>

<style scoped>
.settings-content-inner { flex: 1; display: flex; flex-direction: column; padding: 0; min-height: 0; }

.content-header {
  display: flex; align-items: center;
  padding: 20px 28px 0; flex-shrink: 0; height: 56px;
}
.content-title {
  font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin: 0;
}

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
.agent-card.expanded {
  border-color: var(--ip-primary-400);
  box-shadow: var(--ip-shadow-sm);
}

/* 新建卡片（虚线，仿侧边栏新建对话） */
.new-card {
  border: 1px dashed var(--ip-color-border-default);
  background-color: transparent;
}
.new-card:hover {
  border-color: var(--ip-primary-400);
  background-color: var(--ip-color-bg-tertiary);
}
.new-card.expanded {
  border-style: solid;
  border-color: var(--ip-primary-400);
  background-color: var(--ip-color-bg-secondary);
}

/* 分隔线 */
.list-divider {
  height: 1px;
  background-color: var(--ip-color-border-default);
  margin: 2px 4px;
}

.card-top {
  display: flex;
  align-items: center;
  gap: 12px;
  cursor: pointer;
}

.card-avatar {
  width: 36px; height: 36px; border-radius: var(--ip-radius-md);
  background: linear-gradient(135deg, var(--ip-primary-400), var(--ip-primary-600));
  color: white; display: flex; align-items: center; justify-content: center;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
  flex-shrink: 0;
}
/* 新建卡：内联 + 图标（仿侧栏「新建对话」，无填充图标盒） */
.new-plus {
  flex-shrink: 0;
  color: var(--ip-color-primary-tint-text);
}

.card-body {
  flex: 1; min-width: 0;
  display: flex; flex-direction: column; gap: 3px;
}

.card-name-row { display: flex; align-items: center; gap: 6px; }
.card-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.new-name { color: var(--ip-color-primary-tint-text); }
.new-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); padding-left: 22px; }

.card-file-badge {
  display: inline-flex; align-items: center;
  height: 18px; padding: 0 6px;
  font-size: 9px; font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-primary-tint-text);
  background-color: var(--ip-color-primary-tint-bg);
  border-radius: var(--ip-radius-full);
  font-family: var(--ip-font-mono);
}

.card-meta-row {
  display: flex; align-items: center; gap: 6px;
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

.card-model { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

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
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.card-chevron.rotated {
  transform: rotate(90deg);
  color: var(--ip-primary-600);
}

.card-desc {
  margin: 8px 0 0 48px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}
.card-workspace {
  margin: 4px 0 0 48px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-disabled);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-family: var(--ip-font-mono);
}

/* ===== 展开面板（编辑表单 + 知识库） ===== */
.expand-panel {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--ip-color-border-default);
}

/* 区段（知识库等，caption 标题 + 留白，无嵌套框） */
.region {
  margin-top: 18px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.region-title {
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
  letter-spacing: 0.02em;
}

/* ===== 状态 ===== */
.loading-state { padding: 20px; text-align: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }
.empty-hint { padding: 16px 12px; text-align: center; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
</style>
