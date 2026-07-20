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

import { computed, nextTick, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { useAgentsStore } from "../../stores/agents";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../../stores/projects";
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

// ============================================================================
// 新建会话：Agent 选择逻辑
// ============================================================================

/** Agent 选择浮层是否打开（多个 Agent 时弹出选择） */
const showAgentPicker = ref<boolean>(false);

/** 当前项目可用的 Agent 列表（id + name） */
const projectAgentOptions = computed<Array<{ id: string; name: string }>>(() => {
  const p = projectsStore.current;
  if (!p || !p.agents || p.agents.length === 0) return [];
  return p.agents.map((member) => {
    const agent = agentsStore.byId(member.agent_id);
    return {
      id: member.agent_id,
      name: agent?.name ?? "Unknown",
    };
  });
});

/**
 * 点击「+ 新建会话」按钮（Phase 2 改进版）。
 *
 * Agent 选择策略：
 * - 项目无 Agent 成员 + 非默认项目 → toast 提示并跳转项目设置
 * - 项目无 Agent 成员 + 默认项目   → 回退到 currentId / 第一个 Agent
 * - 项目仅 1 个 Agent 成员         → 直接用该 Agent 创建
 * - 项目有多个 Agent 成员          → 弹出选择浮层
 */
async function onCreate(): Promise<void> {
  const currentProject = projectsStore.current;
  const isDefault = projectsStore.currentId === DEFAULT_PROJECT_ID;
  const projectAgents = currentProject?.agents ?? [];

  // 场景 1：项目无 Agent 成员
  if (projectAgents.length === 0) {
    if (isDefault) {
      // 默认项目：回退到全局 Agent
      const fallbackId =
        agentsStore.currentId ??
        (agentsStore.hasAgents ? agentsStore.agents[0]!.id : null);
      if (!fallbackId) {
        toast.warning("请先创建一个 Agent");
        return;
      }
      await doCreate(fallbackId);
      return;
    }
    // 非默认项目：提示并跳转设置
    toast.warning("请先在项目设置中添加 Agent", { duration: 4000 });
    const id = projectsStore.currentId;
    const routeId = id === DEFAULT_PROJECT_ID ? "default" : id;
    void router.push({ name: "ProjectSettings", params: { projectId: routeId } });
    return;
  }

  // 场景 2：仅 1 个 Agent 成员 → 直接创建
  if (projectAgents.length === 1) {
    await doCreate(projectAgents[0]!.agent_id);
    return;
  }

  // 场景 3：多个 Agent 成员 → 弹出选择浮层
  showAgentPicker.value = true;
}

/** 执行创建会话 */
async function doCreate(agentId: string): Promise<void> {
  showAgentPicker.value = false;
  try {
    const created = await conversationsStore.create(agentId, projectsStore.currentId);
    await nextTick();
    emit("chat:select", created.id);
  } catch {
    toast.error("新建会话失败");
  }
}

/** 取消 Agent 选择 */
function cancelAgentPicker(): void {
  showAgentPicker.value = false;
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
      <!-- Agent 选择浮层（多个 Agent 时显示） -->
      <div v-if="showAgentPicker" class="agent-picker-backdrop" @click="cancelAgentPicker" />
      <Transition name="agent-picker">
        <div v-if="showAgentPicker" class="agent-picker">
          <p class="agent-picker-title">选择 Agent</p>
          <button
            v-for="agent in projectAgentOptions"
            :key="agent.id"
            type="button"
            class="agent-picker-item"
            @click="doCreate(agent.id)"
          >
            {{ agent.name }}
          </button>
        </div>
      </Transition>
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

/* ===== Agent 选择浮层 ===== */
.agent-picker-backdrop {
  position: fixed;
  inset: 0;
  z-index: var(--ip-z-popover, 100);
}

.agent-picker {
  position: absolute;
  bottom: calc(100% + 4px);
  left: var(--ip-spacing-3);
  right: var(--ip-spacing-3);
  z-index: calc(var(--ip-z-popover, 100) + 1);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg, 12px);
  box-shadow: 0 -4px 16px -4px rgba(0, 0, 0, 0.12);
  padding: var(--ip-spacing-2);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.agent-picker-title {
  margin: 0 0 var(--ip-spacing-1);
  padding: var(--ip-spacing-1) var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size, 12px);
  font-weight: var(--ip-font-weight-medium, 500);
  color: var(--ip-color-text-tertiary);
}

.agent-picker-item {
  display: block;
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border: none;
  border-radius: var(--ip-radius-md, 8px);
  background: transparent;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-primary);
  text-align: left;
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.agent-picker-item:hover {
  background: var(--ip-color-bg-hover);
}

.agent-picker-item:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* Agent picker transition */
.agent-picker-enter-active,
.agent-picker-leave-active {
  transition:
    opacity var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.agent-picker-enter-from,
.agent-picker-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
</style>
