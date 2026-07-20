<script setup lang="ts">
// 项目欢迎页（Phase 2: Task 10）
//
// 职责：
//   - 进入项目但未选中会话时的引导界面
//   - 展示项目名、描述
//   - 3 个快捷入口按钮（新建会话 / 管理 Agent / 项目设置）
//   - 最近会话列表（最多 5 条）
//   - 默认项目显示「默认项目」+「未归类到具体项目的会话」
//
// 与 TemplateCards 的区别：
//   - ProjectWelcome = 没有选中任何会话 → 项目级空状态
//   - TemplateCards  = 选中了会话但无消息 → 会话级引导卡片
//
// emits:
//   - create  用户点击「新建会话」时触发

import { computed } from "vue";
import { useRouter } from "vue-router";
import { MessageSquarePlus, Bot, Settings, Clock } from "lucide-vue-next";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../../stores/projects";
import { useConversationsStore } from "../../stores/conversations";
import type { Conversation } from "../../types";

const emit = defineEmits<{
  create: [];
}>();

const projectsStore = useProjectsStore();
const conversationsStore = useConversationsStore();
const router = useRouter();

/** 是否为默认项目 */
const isDefault = computed<boolean>(
  () => projectsStore.currentId === DEFAULT_PROJECT_ID,
);

/** 项目名称 */
const projectName = computed<string>(() => {
  if (isDefault.value) return "默认项目";
  return projectsStore.current?.name ?? "项目";
});

/** 项目描述 */
const projectDescription = computed<string>(() => {
  if (isDefault.value) return "未归类到具体项目的会话";
  return projectsStore.current?.description?.trim() || "开始你的项目协作";
});

/** 项目 Agent 成员数量 */
const agentCount = computed<number>(() => {
  return projectsStore.current?.agents?.length ?? 0;
});

/** 最近会话（最多 5 条） */
const recentConversations = computed<Conversation[]>(() => {
  return conversationsStore
    .listForProject(projectsStore.currentId)
    .slice(0, 5);
});

/** 是否有最近会话 */
const hasRecent = computed<boolean>(() => recentConversations.value.length > 0);

/** 新建会话 */
function onCreate(): void {
  emit("create");
}

/** 跳转到 Agent 管理 */
function goToAgentManager(): void {
  void router.push({ name: "AgentManager" });
}

/** 跳转到项目设置 */
function goToSettings(): void {
  if (isDefault.value) {
    // 默认项目没有设置页，跳转到 Agent 管理
    void router.push({ name: "AgentManager" });
    return;
  }
  const id = projectsStore.currentId;
  if (id && id !== DEFAULT_PROJECT_ID) {
    void router.push({ name: "ProjectSettings", params: { projectId: id } });
  }
}

/** 点击最近会话 */
function onSelectConversation(conv: Conversation): void {
  conversationsStore.setCurrent(conv.id);
}

/** 格式化时间为简短相对时间 */
function formatTime(isoStr: string): string {
  if (!isoStr) return "";
  const date = new Date(isoStr);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMin = Math.floor(diffMs / 60000);
  const diffHour = Math.floor(diffMin / 60);
  const diffDay = Math.floor(diffHour / 24);

  if (diffMin < 1) return "刚刚";
  if (diffMin < 60) return `${diffMin} 分钟前`;
  if (diffHour < 24) return `${diffHour} 小时前`;
  if (diffDay < 7) return `${diffDay} 天前`;
  return date.toLocaleDateString("zh-CN", { month: "short", day: "numeric" });
}
</script>

<template>
  <div class="project-welcome">
    <!-- 项目信息区 -->
    <div class="welcome-section">
      <h1 class="project-name">{{ projectName }}</h1>
      <p class="project-desc">{{ projectDescription }}</p>
    </div>

    <!-- 快捷入口 -->
    <div class="quick-actions">
      <button type="button" class="action-card action-card-primary" @click="onCreate">
        <span class="action-icon action-icon-primary" aria-hidden="true">
          <MessageSquarePlus :size="20" />
        </span>
        <span class="action-label">
          <span class="action-title">新建会话</span>
          <span class="action-hint">开始对话</span>
        </span>
      </button>

      <button type="button" class="action-card" @click="goToAgentManager">
        <span class="action-icon" aria-hidden="true">
          <Bot :size="20" />
        </span>
        <span class="action-label">
          <span class="action-title">管理 Agent</span>
          <span class="action-hint">{{ agentCount > 0 ? `${agentCount} 个成员` : "去配置" }}</span>
        </span>
      </button>

      <button type="button" class="action-card" @click="goToSettings">
        <span class="action-icon" aria-hidden="true">
          <Settings :size="20" />
        </span>
        <span class="action-label">
          <span class="action-title">{{ isDefault ? "Agent 管理" : "项目设置" }}</span>
          <span class="action-hint">{{ isDefault ? "去配置" : "配置项目" }}</span>
        </span>
      </button>
    </div>

    <!-- 最近会话 -->
    <div v-if="hasRecent" class="recent-section">
      <div class="recent-header">
        <Clock :size="14" aria-hidden="true" />
        <span>最近会话</span>
      </div>
      <div class="recent-list">
        <button
          v-for="conv in recentConversations"
          :key="conv.id"
          type="button"
          class="recent-item"
          @click="onSelectConversation(conv)"
        >
          <span class="recent-item-title">{{ conv.title || "未命名会话" }}</span>
          <span class="recent-item-time">{{ formatTime(conv.updated_at) }}</span>
        </button>
      </div>
    </div>

    <!-- 无会话提示 -->
    <div v-else class="empty-hint">
      <p>暂无会话，点击「新建会话」开始</p>
    </div>
  </div>
</template>

<style scoped>
.project-welcome {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px var(--ip-spacing-6);
  gap: var(--ip-spacing-8);
  background: var(--ip-color-bg-primary);
  overflow-y: auto;
}

/* ===== 项目信息 ===== */
.welcome-section {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ip-spacing-2);
  max-width: 480px;
  width: 100%;
  text-align: center;
}

.project-name {
  margin: 0;
  font-size: var(--ip-text-h1-size, 28px);
  font-weight: var(--ip-font-weight-bold, 700);
  color: var(--ip-color-text-primary);
  letter-spacing: -0.02em;
  line-height: 1.2;
}

.project-desc {
  margin: 0;
  font-size: var(--ip-text-body-size, 15px);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}

/* ===== 快捷入口 ===== */
.quick-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: var(--ip-spacing-3);
  max-width: 600px;
  width: 100%;
}

.action-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ip-spacing-2);
  width: 140px;
  padding: var(--ip-spacing-4);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg, 12px);
  background: var(--ip-color-bg-secondary);
  cursor: pointer;
  font-family: inherit;
  text-align: center;
  transition:
    border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    box-shadow var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.action-card:hover {
  border-color: var(--ip-color-border-strong);
  background: var(--ip-color-bg-tertiary);
  transform: translateY(-2px);
  box-shadow: 0 4px 12px -4px rgba(0, 0, 0, 0.08);
}

.action-card:active {
  transform: translateY(0);
}

.action-card:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.action-card-primary {
  border-color: var(--ip-primary-600, #2563eb);
  background: var(--ip-primary-50, #eff6ff);
}

.action-card-primary:hover {
  border-color: var(--ip-primary-700, #1d4ed8);
  background: var(--ip-primary-100, #dbeafe);
}

.action-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border-radius: var(--ip-radius-md, 8px);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
  flex-shrink: 0;
}

.action-icon-primary {
  background: var(--ip-primary-600, #2563eb);
  color: var(--ip-color-text-on-primary, #ffffff);
}

.action-label {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.action-title {
  font-size: var(--ip-text-body-sm-size, 13px);
  font-weight: var(--ip-font-weight-semibold, 600);
  color: var(--ip-color-text-primary);
  line-height: 1.3;
}

.action-hint {
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-tertiary);
  line-height: 1.3;
}

/* ===== 最近会话 ===== */
.recent-section {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  max-width: 480px;
  width: 100%;
}

.recent-header {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size, 12px);
  font-weight: var(--ip-font-weight-medium, 500);
  color: var(--ip-color-text-tertiary);
  padding: 0 var(--ip-spacing-1);
}

.recent-list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.recent-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md, 8px);
  background: var(--ip-color-bg-secondary);
  cursor: pointer;
  font-family: inherit;
  text-align: left;
  width: 100%;
  transition: var(--ip-transition-colors);
}

.recent-item:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}

.recent-item:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.recent-item-title {
  font-size: var(--ip-text-body-sm-size, 13px);
  font-weight: var(--ip-font-weight-medium, 500);
  color: var(--ip-color-text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.recent-item-time {
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  flex-shrink: 0;
}

/* ===== 空提示 ===== */
.empty-hint {
  max-width: 480px;
  width: 100%;
  text-align: center;
}

.empty-hint p {
  margin: 0;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-tertiary);
}

/* ===== 响应式：窄屏 ===== */
@media (max-width: 480px) {
  .quick-actions {
    flex-direction: column;
    align-items: stretch;
  }

  .action-card {
    width: 100%;
    flex-direction: row;
    text-align: left;
  }
}
</style>
