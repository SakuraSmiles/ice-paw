<script setup lang="ts">
// ProjectDetail.vue — 项目详情：会话 / 成员 / 文件 三个 Tab
import { ref, computed, onMounted, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useProjectStore } from "../stores/project";
import { useAgentStore } from "../stores/agent";
import { useChatStore } from "../stores/chat";
import type { ProjectAgent } from "../types";
import AgentPicker from "../components/chat/AgentPicker.vue";

const route = useRoute();
const router = useRouter();
const projectStore = useProjectStore();
const agent = useAgentStore();
const chat = useChatStore();

const projectId = computed(() => String(route.params.id));
const project = computed(() => projectStore.getById(projectId.value));

const tab = ref<"conversations" | "members" | "files">("conversations");
const showPicker = ref(false);

// 项目内会话（chat.conversations 已含 project_id，纯客户端过滤）
const projectConversations = computed(() =>
  chat.conversations.filter((c) => c.project_id === projectId.value),
);

const members = computed<ProjectAgent[]>(() => project.value?.agents ?? []);
const memberAgentIds = computed(() => members.value.map((m) => m.agent_id));
// 可添加的 agent（非成员）
const candidateAgents = computed(() =>
  agent.list.filter((a) => !memberAgentIds.value.includes(a.id)),
);

async function ensureLoaded() {
  await projectStore.load();
  agent.load();
  chat.loadConversations();
}

onMounted(ensureLoaded);
// 路由 param 变化（如侧栏直接切到另一个项目）时重载
watch(projectId, ensureLoaded);

function openConversation(convId: string) {
  chat.selectConversation(convId);
  router.push("/");
}

async function onPickAgent(agentId: string) {
  showPicker.value = false;
  try {
    const conv = await chat.createConversation(agentId, projectId.value);
    chat.selectConversation(conv.id);
    router.push("/");
  } catch (e) {
    console.error("在项目中新建会话失败:", e);
  }
}

async function addMember(agentId: string) {
  try {
    await projectStore.load(); // 确保最新
    // 直接调 bridge 的 addAgent，store 没有封装单成员添加
    const { bridge } = await import("../api/bridge");
    await bridge.projects.addAgent(projectId.value, agentId, "member");
    await projectStore.load(true);
  } catch (e) {
    console.error("添加成员失败:", e);
  }
}

async function removeMember(agentId: string) {
  try {
    const { bridge } = await import("../api/bridge");
    await bridge.projects.removeAgent(projectId.value, agentId);
    await projectStore.load(true);
  } catch (e) {
    console.error("移除成员失败:", e);
  }
}

async function openInExplorer() {
  const ws = project.value?.workspace_path;
  if (!ws) return;
  try {
    await revealItemInDir(ws);
  } catch (e) {
    console.error("打开目录失败:", e);
  }
}

function agentName(id: string): string {
  return agent.getById(id)?.name ?? "未知";
}

function timeAgo(dateStr: string): string {
  const d = new Date(dateStr);
  const diff = Date.now() - d.getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "刚刚";
  if (mins < 60) return `${mins}分钟前`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}小时前`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}天前`;
  return dateStr.slice(0, 10);
}
</script>

<template>
  <div class="page-inner">
    <!-- 顶部：返回 + 项目名 + 描述 -->
    <div class="detail-header">
      <button class="back-btn" title="返回项目列表" @click="router.push('/projects')">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="19" y1="12" x2="5" y2="12" /><polyline points="12 19 5 12 12 5" />
        </svg>
      </button>
      <div v-if="project" class="header-info">
        <div class="header-title-row">
          <span class="header-avatar">{{ project.name.charAt(0) }}</span>
          <h2 class="header-title">{{ project.name }}</h2>
        </div>
        <p v-if="project.description" class="header-desc">{{ project.description }}</p>
      </div>
      <div v-else class="header-info">
        <h2 class="header-title">加载中…</h2>
      </div>
    </div>

    <!-- Tab 切换 -->
    <div class="tab-bar">
      <button class="tab-btn" :class="{ active: tab === 'conversations' }" @click="tab = 'conversations'">
        会话 <span class="tab-count">{{ projectConversations.length }}</span>
      </button>
      <button class="tab-btn" :class="{ active: tab === 'members' }" @click="tab = 'members'">
        成员 <span class="tab-count">{{ members.length }}</span>
      </button>
      <button class="tab-btn" :class="{ active: tab === 'files' }" @click="tab = 'files'">文件</button>
    </div>

    <!-- 会话 Tab -->
    <div v-if="tab === 'conversations'" class="tab-body">
      <button class="new-conv-btn" @click="showPicker = true">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        新建会话
      </button>

      <div v-if="projectConversations.length === 0" class="empty-hint">
        项目内还没有会话，点上方「新建会话」开始（agent 将在项目源码目录下工作）
      </div>
      <div v-else class="conv-list">
        <button
          v-for="conv in projectConversations"
          :key="conv.id"
          class="conv-item"
          @click="openConversation(conv.id)"
        >
          <div class="conv-item-title">{{ conv.title || "新对话" }}</div>
          <div class="conv-item-meta">
            <span class="conv-agent-tag">{{ agentName(conv.agent_id) }}</span>
            <span class="conv-time">{{ timeAgo(conv.updated_at) }}</span>
          </div>
        </button>
      </div>
    </div>

    <!-- 成员 Tab -->
    <div v-else-if="tab === 'members'" class="tab-body">
      <div v-if="members.length === 0" class="empty-hint">项目暂无成员</div>
      <div class="member-list">
        <div v-for="m in members" :key="m.agent_id" class="member-row">
          <div class="member-avatar">{{ agentName(m.agent_id).charAt(0) }}</div>
          <div class="member-info">
            <div class="member-name">{{ agentName(m.agent_id) }}</div>
            <div class="member-role">{{ m.role }}</div>
          </div>
          <button class="member-remove" title="移除成员" @click="removeMember(m.agent_id)">移除</button>
        </div>
      </div>

      <div v-if="candidateAgents.length > 0" class="add-region">
        <div class="region-title">添加成员</div>
        <div class="add-chips">
          <button
            v-for="a in candidateAgents"
            :key="a.id"
            class="add-chip"
            @click="addMember(a.id)"
          >
            + {{ a.name }}
          </button>
        </div>
      </div>
      <div v-else-if="members.length > 0" class="empty-hint small">全部智能体已加入</div>
    </div>

    <!-- 文件 Tab -->
    <div v-else class="tab-body">
      <div v-if="project && project.workspace_path" class="file-panel">
        <div class="region-title">源码目录</div>
        <div class="ws-path" :title="project.workspace_path">{{ project.workspace_path }}</div>
        <p class="field-hint">项目内会话的文件/代码工具（read_file / write_file / run_command / git / search_files）会切换到此目录。agent 个人工作区与知识库不受影响。</p>
        <button class="btn btn-secondary btn-sm" @click="openInExplorer">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 15v2a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2h2" />
            <polyline points="15 3 21 3 21 9" /><line x1="10" y1="14" x2="21" y2="3" />
          </svg>
          在文件管理器中打开
        </button>
      </div>
      <div v-else class="empty-hint">
        未绑定源码目录。前往项目列表删除并重建，或后续支持编辑后再绑定。
      </div>
    </div>

    <!-- 成员限定的 AgentPicker -->
    <AgentPicker
      v-if="showPicker"
      :agent-ids="memberAgentIds.length ? memberAgentIds : undefined"
      @select="onPickAgent"
      @close="showPicker = false"
    />
  </div>
</template>

<style scoped>
.page-inner { flex: 1; display: flex; flex-direction: column; min-height: 0; }

.detail-header {
  display: flex; align-items: flex-start; gap: 12px;
  padding: 16px 24px 12px; flex-shrink: 0;
}
.back-btn {
  display: flex; align-items: center; justify-content: center;
  width: 32px; height: 32px; margin-top: 4px;
  border-radius: var(--ip-radius-md); cursor: pointer;
  background: none; border: none; color: var(--ip-color-text-secondary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.back-btn:hover { background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }
.header-info { flex: 1; min-width: 0; }
.header-title-row { display: flex; align-items: center; gap: 10px; }
.header-avatar {
  width: 32px; height: 32px; border-radius: var(--ip-radius-md); flex-shrink: 0;
  background: linear-gradient(135deg, var(--ip-primary-400), var(--ip-primary-600));
  color: white; display: flex; align-items: center; justify-content: center;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
}
.header-title { font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); margin: 0; }
.header-desc { margin: 4px 0 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

/* ===== Tab 栏 ===== */
.tab-bar {
  display: flex; gap: 4px;
  padding: 0 20px;
  border-bottom: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}
.tab-btn {
  position: relative;
  display: inline-flex; align-items: center; gap: 6px;
  height: 38px; padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  background: none; border: none; cursor: pointer; font-family: inherit;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.tab-btn:hover { color: var(--ip-color-text-primary); }
.tab-btn.active { color: var(--ip-primary-600); font-weight: var(--ip-font-weight-medium); }
.tab-btn.active::after {
  content: ''; position: absolute; left: 8px; right: 8px; bottom: -1px;
  height: 2px; background: var(--ip-primary-500); border-radius: 1px;
}
.tab-count {
  font-size: 10px; line-height: 16px; padding: 0 5px;
  color: var(--ip-color-text-tertiary);
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-full);
}

.tab-body {
  flex: 1; overflow-y: auto;
  padding: 16px 24px 24px;
  display: flex; flex-direction: column; gap: 8px;
  min-height: 0;
}

.new-conv-btn {
  display: inline-flex; align-items: center; gap: 6px; align-self: flex-start;
  height: 34px; padding: 0 14px;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-primary-tint-text);
  background-color: var(--ip-color-primary-tint-bg);
  border: 1px dashed var(--ip-primary-400);
  border-radius: var(--ip-radius-md); cursor: pointer; font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.new-conv-btn:hover { background-color: var(--ip-color-primary-tint-bg); }

/* 会话列表 */
.conv-list { display: flex; flex-direction: column; gap: 2px; }
.conv-item {
  display: flex; flex-direction: column; gap: 3px;
  width: 100%; padding: 10px 12px; text-align: left;
  border-radius: var(--ip-radius-lg); cursor: pointer;
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.conv-item:hover { border-color: var(--ip-primary-300); }
.conv-item-title { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); }
.conv-item-meta { display: flex; align-items: center; gap: 8px; }
.conv-agent-tag { font-size: 11px; color: var(--ip-primary-600); font-weight: var(--ip-font-weight-medium); }
.conv-time { font-size: 11px; color: var(--ip-color-text-disabled); margin-left: auto; }

/* 成员列表 */
.member-list { display: flex; flex-direction: column; gap: 4px; }
.member-row {
  display: flex; align-items: center; gap: 12px;
  padding: 10px 12px;
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
}
.member-avatar {
  width: 32px; height: 32px; border-radius: var(--ip-radius-md); flex-shrink: 0;
  background: linear-gradient(135deg, var(--ip-primary-400), var(--ip-primary-600));
  color: white; display: flex; align-items: center; justify-content: center;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
}
.member-info { flex: 1; min-width: 0; }
.member-name { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); }
.member-role { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.member-remove {
  height: 26px; padding: 0 10px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  background: none; border: 1px solid transparent; border-radius: var(--ip-radius-md);
  cursor: pointer; font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.member-remove:hover { color: var(--ip-danger-text); border-color: var(--ip-danger-border); background: var(--ip-danger-bg); }

.add-region { margin-top: 16px; display: flex; flex-direction: column; gap: 8px; }
.region-title {
  font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary); letter-spacing: 0.02em;
}
.add-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.add-chip {
  height: 28px; padding: 0 12px;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px dashed var(--ip-color-border-default);
  border-radius: var(--ip-radius-full); cursor: pointer; font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.add-chip:hover { border-color: var(--ip-primary-400); color: var(--ip-color-primary-tint-text); }

/* 文件面板 */
.file-panel { display: flex; flex-direction: column; gap: 8px; }
.ws-path {
  padding: 10px 12px;
  font-family: var(--ip-font-mono); font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-md);
  word-break: break-all;
}
.field-hint { margin: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); line-height: 1.5; }

.btn {
  display: inline-flex; align-items: center; gap: 6px; align-self: flex-start;
  border-radius: var(--ip-radius-md); cursor: pointer; font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-sm { height: 30px; padding: 0 14px; font-size: var(--ip-text-body-sm-size); }
.btn-secondary {
  background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary);
  border: 1px solid var(--ip-color-border-default);
}
.btn-secondary:hover { border-color: var(--ip-primary-300); color: var(--ip-primary-600); }

.empty-hint { padding: 16px 12px; text-align: center; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.empty-hint.small { padding: 8px 12px; text-align: left; }
</style>
