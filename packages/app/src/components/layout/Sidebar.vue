<script setup lang="ts">
// 侧边栏（拼装版）
//
// 职责：
//   - 顶部：AgentSelector（Agent 切换 + 管理入口）
//   - 中部：ConversationList（pinned 分组 + 列表）
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
import { useAgentsStore } from "../../stores/agents";
import { useConversationsStore } from "../../stores/conversations";
import { useContextMenu } from "../../composables/useContextMenu";
import { useToast } from "../../composables/useToast";
import type { Conversation } from "../../types";
import AgentSelector from "../sidebar/AgentSelector.vue";
import ConversationList from "../sidebar/ConversationList.vue";
import NewChatButton from "../sidebar/NewChatButton.vue";
import ContextMenu from "../sidebar/ContextMenu.vue";

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const ctxMenu = useContextMenu();
const toast = useToast();

const emit = defineEmits<{
  "chat:select": [conversationId: string | null];
}>();

// ============================================================================
// 生命周期：启动监听 Agent 切换（在确保 agents 加载完成后调用更稳妥）
// ============================================================================

onMounted(() => {
  conversationsStore.watchAgentChange();
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

/** 点击「+ 新建会话」按钮 */
async function onCreate(): Promise<void> {
  const agentId = agentsStore.currentId;
  if (!agentId) {
    toast.warning("请先选择或创建一个 Agent");
    return;
  }
  try {
    const created = await conversationsStore.create(agentId);
    // 等 Vue 把 create() 写入的 currentId 推到 ChatPage 的响应式依赖，
    // 再 emit chat:select，避免父组件在 currentId 同步之前读到旧值。
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
  // 先切换选中（与桌面客户端常见行为一致：右键即选中）
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

/** 切换置顶（包装 store action + 错误提示） */
async function onTogglePin(conv: Conversation): Promise<void> {
  try {
    await conversationsStore.pin(conv.id, !conv.pinned);
  } catch {
    toast.error(conv.pinned ? "取消置顶失败" : "置顶失败");
  }
}

/** 删除会话（包装 store action + 错误提示 + 切换通知） */
async function onDelete(conv: Conversation): Promise<void> {
  try {
    const newCurrentId = await conversationsStore.delete(conv.id);
    // 通知父组件当前会话可能已变化
    emit("chat:select", newCurrentId);
  } catch {
    toast.error("删除失败");
  }
}

// ============================================================================
// 重命名
// ============================================================================

/** 双击 → 进入重命名态 */
function onRequestRename(conv: Conversation): void {
  conversationsStore.requestRename(conv.id);
}

/** 重命名提交（store.rename 失败时由 toast 提示并退出编辑态） */
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

/** 重命名取消 */
function onCancelRename(): void {
  conversationsStore.cancelRename();
}
</script>

<template>
  <aside class="sidebar">
    <!-- 顶部：Agent 选择器 -->
    <div class="sidebar-top">
      <AgentSelector />
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
  /* 侧边栏与主区轻微分层：使用 secondary 作为底色 */
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
}
</style>