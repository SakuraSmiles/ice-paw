<script setup lang="ts">
// 侧边栏（Phase 2 重构版）
//
// 职责：
//   - 顶部：ProjectSelector（项目切换 + 管理入口）
//   - 中部：ConversationList（pinned 分组 + 列表，按项目加载）
//   - 底部：NewChatButton
//   - 浮层：ContextMenu（右键菜单）+ InlineRename（由 ConversationItem 内部渲染）
//
// 事件流：
//   - 选中会话   → conversationsStore.setCurrent + emit chat:select
//   - 新建会话   → conversationsStore.create + emit chat:select
//   - 右键菜单   → 重命名/置顶/删除
//   - 双击标题   → 进入重命名态
//
// emits:
//   - chat:select(conversationId)  当前选中的会话 ID 变化时通知父组件

import { nextTick, onMounted } from "vue";
import { useRouter } from "vue-router";
import { useAgentsStore } from "../../stores/agents";
import { useProjectsStore } from "../../stores/projects";
import { useConversationsStore } from "../../stores/conversations";
import { useContextMenu } from "../../composables/useContextMenu";
import { useToast } from "../../composables/useToast";
import type { Conversation } from "../../types";
import ProjectSelector from "../sidebar/ProjectSelector.vue";
import ConversationList from "../sidebar/ConversationList.vue";
import NewChatButton from "../sidebar/NewChatButton.vue";
import ContextMenu from "../sidebar/ContextMenu.vue";

const agentsStore = useAgentsStore();
const projectsStore = useProjectsStore();
const conversationsStore = useConversationsStore();
const ctxMenu = useContextMenu();
const toast = useToast();
const router = useRouter();

/** 跳转到模板管理页 */
function goToTemplateManager(): void {
  void router.push({ name: "TemplateManager" });
}

const emit = defineEmits<{
  "chat:select": [conversationId: string | null];
}>();

// ============================================================================
// 生命周期：启动监听项目切换
// ============================================================================

onMounted(() => {
  conversationsStore.watchProjectChange();
});

// ============================================================================
// 会话选中
// ============================================================================

/** 点击会话项 → 选中 */
function onSelect(conv: Conversation): void {
  conversationsStore.setCurrent(conv.id);
  emit("chat:select", conv.id);
}

// ============================================================================
// 新建会话
// ============================================================================

/**
 * 点击「+ 新建会话」按钮。
 * Phase 2: 不再依赖 agentsStore.currentId 作为侧边栏的核心维度。
 * - 如果当前项目有 Agent 成员，优先使用 lead agent
 * - 否则使用 agentsStore.currentId 或第一个 Agent
 */
async function onCreate(): Promise<void> {
  // 获取当前项目的 Agent 成员
  const currentProject = projectsStore.current;
  let agentId: string | null = null;

  if (currentProject && currentProject.agents.length > 0) {
    // 优先使用 lead agent
    const lead = currentProject.agents.find((a) => a.role === "lead");
    agentId = lead?.agent_id ?? currentProject.agents[0]!.agent_id;
  }

  // 回退到 agentsStore
  if (!agentId) {
    agentId = agentsStore.currentId;
  }

  // 回退到第一个 Agent
  if (!agentId && agentsStore.hasAgents) {
    agentId = agentsStore.agents[0]!.id;
  }

  if (!agentId) {
    toast.warning("请先创建一个 Agent");
    return;
  }

  try {
    const created = await conversationsStore.create(agentId, projectsStore.currentId);
    await nextTick();
    emit("chat:select", created.id);
  } catch {
    toast.error("新建会话失败");
  }
}

// ============================================================================
// 右键菜单
// ============================================================================

/** 右键会话项：构建菜单项并打开浮层 */
function onContextMenu(event: MouseEvent, conv: Conversation): void {
  conversationsStore.setCurrent(conv.id);
  emit("chat:select", conv.id);

  const items = [
    {
      label: "重命名",
      handler: () => {
        conversationsStore.requestRename(conv.id);
      },
    },
    {
      label: conv.pinned ? "取消置顶" : "置顶",
      handler: () => {
        void onTogglePin(conv);
      },
    },
    {
      label: "删除",
      danger: true,
      handler: () => {
        void onDelete(conv);
      },
    },
  ];
  ctxMenu.openMenu(event.clientX, event.clientY, items);
}

/** 切换置顶 */
async function onTogglePin(conv: Conversation): Promise<void> {
  try {
    await conversationsStore.pin(conv.id, !conv.pinned);
  } catch {
    toast.error(conv.pinned ? "取消置顶失败" : "置顶失败");
  }
}

/** 删除会话 */
async function onDelete(conv: Conversation): Promise<void> {
  try {
    const newCurrentId = await conversationsStore.delete(conv.id);
    emit("chat:select", newCurrentId);
  } catch {
    toast.error("删除失败");
  }
}

// ============================================================================
// 重命名
// ============================================================================

function onRequestRename(conv: Conversation): void {
  conversationsStore.requestRename(conv.id);
}

async function onCommitRename(title: string): Promise<void> {
  const renamingId = conversationsStore.renamingId;
  if (!renamingId) return;
  try {
    await conversationsStore.rename(renamingId, title);
  } catch {
    toast.error("重命名失败");
    conversationsStore.cancelRename();
  }
}

function onCancelRename(): void {
  conversationsStore.cancelRename();
}
</script>

<template>
  <aside class="sidebar">
    <!-- 顶部：项目选择器 -->
    <div class="sidebar-top">
      <ProjectSelector />
    </div>

    <!-- 中部：会话列表（滚动区域） -->
    <div class="sidebar-middle">
      <ConversationList
        @select="onSelect"
        @contextmenu="onContextMenu"
        @request-rename="onRequestRename"
        @commit-rename="onCommitRename"
        @cancel-rename="onCancelRename"
      />
    </div>

    <!-- 底部：新建会话按钮 -->
    <div class="sidebar-bottom">
      <NewChatButton @create="onCreate" />
      <!-- 模板管理入口 -->
      <button
        type="button"
        class="sidebar-link"
        :aria-label="'管理模板'"
        @click="goToTemplateManager"
      >
        管理模板 →
      </button>
    </div>

    <!-- 全局右键菜单浮层（Teleport 到 body） -->
    <ContextMenu />
  </aside>
</template>

<style scoped>
.sidebar {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  background: var(--ip-color-bg-secondary);
  border-right: 1px solid var(--ip-color-border-default);
  overflow: hidden;
}

.sidebar-top {
  flex: 0 0 auto;
}

.sidebar-middle {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.sidebar-bottom {
  flex: 0 0 auto;
  padding: var(--ip-spacing-3) var(--ip-spacing-3);
  border-top: 1px solid var(--ip-color-border-default);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.sidebar-link {
  display: block;
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: transparent;
  border: 0;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  text-align: left;
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.sidebar-link:hover {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}
</style>
