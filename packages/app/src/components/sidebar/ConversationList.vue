<script setup lang="ts">
// 会话列表
//
// 职责：
//   - 从 conversationsStore 读取当前 Agent 的会话列表
//   - 渲染「已置顶」分组 + 未置顶列表
//   - 无数据时显示「暂无会话」提示
//
// props: 无（直接读 conversationsStore）
//
// emits:
//   - select              点击项 → 选中
//   - contextmenu         右键项 → 弹出菜单（外层 useContextMenu）
//   - requestRename       双击项 → 进入重命名
//   - commitRename(title) 重命名提交
//   - cancelRename        重命名取消

import { computed } from "vue";
import { MessageSquare, Pin } from "lucide-vue-next";
import { useConversationsStore } from "../../stores/conversations";
import { useAgentsStore } from "../../stores/agents";
import type { Conversation } from "../../types";
import ConversationItem from "./ConversationItem.vue";

const conversationsStore = useConversationsStore();
const agentsStore = useAgentsStore();

/** 当前 Agent 的已置顶会话（响应式） */
const pinnedList = computed<Conversation[]>(() => {
  const agentId = agentsStore.currentId;
  if (!agentId) return [];
  return conversationsStore.pinned(agentId);
});

/** 当前 Agent 的未置顶会话（响应式） */
const unpinnedList = computed<Conversation[]>(() => {
  const agentId = agentsStore.currentId;
  if (!agentId) return [];
  return conversationsStore.unpinned(agentId);
});

/** 列表是否为空 */
const isEmpty = computed<boolean>(
  () => pinnedList.value.length === 0 && unpinnedList.value.length === 0,
);

// ============================================================================
// 事件转发（透传到外层 Sidebar 处理 store actions）
// ============================================================================

function onSelect(conv: Conversation): void {
  emit("select", conv);
}
function onContextmenu(event: MouseEvent, conv: Conversation): void {
  emit("contextmenu", event, conv);
}
function onRequestRename(conv: Conversation): void {
  emit("requestRename", conv);
}
function onCommitRename(title: string): void {
  emit("commitRename", title);
}
function onCancelRename(): void {
  emit("cancelRename");
}

const emit = defineEmits<{
  select: [conv: Conversation];
  contextmenu: [event: MouseEvent, conv: Conversation];
  requestRename: [conv: Conversation];
  commitRename: [title: string];
  cancelRename: [];
}>();
</script>

<template>
  <div class="conv-list">
    <!-- 空状态 -->
    <div v-if="isEmpty" class="empty-hint">
      <MessageSquare :size="20" class="empty-hint-icon" aria-hidden="true" />
      <span class="empty-hint-text">暂无会话</span>
      <span class="empty-hint-hint">点击下方按钮开始新对话</span>
    </div>

    <!-- 有数据 -->
    <template v-else>
      <!-- 已置顶分组 -->
      <template v-if="pinnedList.length > 0">
        <div class="group-label">
          <Pin :size="12" class="group-label-icon" aria-hidden="true" />
          <span>已置顶</span>
          <span class="group-label-count">{{ pinnedList.length }}</span>
        </div>
        <ConversationItem
          v-for="conv in pinnedList"
          :key="conv.id"
          :conv="conv"
          :active="conversationsStore.currentId === conv.id"
          :renaming="conversationsStore.renamingId === conv.id"
          @select="onSelect"
          @contextmenu="onContextmenu"
          @request-rename="onRequestRename"
          @commit-rename="onCommitRename"
          @cancel-rename="onCancelRename"
        />
      </template>
      <!-- 未置顶列表 -->
      <template v-if="unpinnedList.length > 0">
        <div v-if="pinnedList.length > 0" class="group-label">
          <span>所有会话</span>
          <span class="group-label-count">{{ unpinnedList.length }}</span>
        </div>
        <ConversationItem
          v-for="conv in unpinnedList"
          :key="conv.id"
          :conv="conv"
          :active="conversationsStore.currentId === conv.id"
          :renaming="conversationsStore.renamingId === conv.id"
          @select="onSelect"
          @contextmenu="onContextmenu"
          @request-rename="onRequestRename"
          @commit-rename="onCommitRename"
          @cancel-rename="onCancelRename"
        />
      </template>
    </template>
  </div>
</template>

<style scoped>
.conv-list {
  display: flex;
  flex-direction: column;
  padding: var(--ip-spacing-1);
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.empty-hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-8) var(--ip-spacing-4);
  user-select: none;
}

.empty-hint-icon {
  color: var(--ip-color-text-tertiary);
  opacity: 0.6;
}

.empty-hint-text {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}

.empty-hint-hint {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.group-label {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-size: 10px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
  padding: var(--ip-spacing-3) var(--ip-spacing-3) var(--ip-spacing-1);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  user-select: none;
}

.group-label-icon {
  color: var(--ip-color-text-tertiary);
}

.group-label-count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 var(--ip-spacing-1);
  font-size: 10px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-tertiary);
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-full);
  letter-spacing: 0;
  text-transform: none;
}
</style>
