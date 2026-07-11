<script setup lang="ts">
// 单个会话项
//
// 职责：
//   - 展示会话标题 + 相对时间 + 置顶图标
//   - 双击进入重命名态
//   - 右键弹出菜单（外层用 useContextMenu 打开）
//   - active 状态高亮
//
// props:
//   - conv      会话实体
//   - active    是否为当前选中
//   - renaming  是否处于重命名态（true 时显示 InlineRename）
//
// emits:
//   - select              点击 → 选中
//   - contextmenu         右键 → 弹出菜单（转发原始 MouseEvent）
//   - requestRename       双击 → 进入重命名
//   - commitRename(title) 重命名提交
//   - cancelRename        重命名取消

import type { Conversation } from "../../types";
import InlineRename from "./InlineRename.vue";

const props = defineProps<{
  conv: Conversation;
  active: boolean;
  renaming: boolean;
}>();

const emit = defineEmits<{
  select: [conv: Conversation];
  contextmenu: [event: MouseEvent, conv: Conversation];
  requestRename: [conv: Conversation];
  commitRename: [title: string];
  cancelRename: [];
}>();

/**
 * 将 ISO 时间字符串格式化为相对时间（中文）。
 * 例：「刚刚」「5 分钟前」「2 小时前」「3 天前」「1 周前」「4 个月前」「2 年前」。
 */
function formatRelative(iso: string): string {
  try {
    const target = new Date(iso).getTime();
    if (Number.isNaN(target)) return iso;
    const now = Date.now();
    const diffMs = now - target;
    if (diffMs < 0) return "刚刚";

    const sec = Math.floor(diffMs / 1000);
    if (sec < 60) return "刚刚";
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min} 分钟前`;
    const hour = Math.floor(min / 60);
    if (hour < 24) return `${hour} 小时前`;
    const day = Math.floor(hour / 24);
    if (day < 7) return `${day} 天前`;
    const week = Math.floor(day / 7);
    if (week < 4) return `${week} 周前`;
    const month = Math.floor(day / 30);
    if (month < 12) return `${month} 个月前`;
    const year = Math.floor(day / 365);
    return `${year} 年前`;
  } catch {
    return iso;
  }
}

/** 点击 → 选中 */
function onClick(): void {
  if (props.renaming) return; // 重命名态下不切换选中
  emit("select", props.conv);
}

/** 右键：阻止默认菜单 + 转发事件 */
function onContextmenu(e: MouseEvent): void {
  e.preventDefault();
  emit("contextmenu", e, props.conv);
}

/** 双击 → 进入重命名 */
function onDblclick(): void {
  emit("requestRename", props.conv);
}

/** 重命名提交 */
function onCommitRename(title: string): void {
  emit("commitRename", title);
}

/** 重命名取消 */
function onCancelRename(): void {
  emit("cancelRename");
}
</script>

<template>
  <div
    :class="['conv-item', { 'conv-item-active': active, 'conv-item-renaming': renaming }]"
    @click="onClick"
    @contextmenu="onContextmenu"
    @dblclick="onDblclick"
  >
    <div class="conv-content">
      <InlineRename
        v-if="renaming"
        :model-value="conv.title"
        :editing="renaming"
        @commit="onCommitRename"
        @cancel="onCancelRename"
      />
      <div v-else class="conv-title-row">
        <span v-if="conv.pinned" class="conv-pin" title="已置顶">置顶</span>
        <span class="conv-title">{{ conv.title || "新会话" }}</span>
      </div>
      <div class="conv-time">{{ formatRelative(conv.updated_at) }}</div>
    </div>
  </div>
</template>

<style scoped>
.conv-item {
  padding: 10px 14px;
  border-radius: 6px;
  cursor: pointer;
  transition: background 80ms ease;
  user-select: none;
}

.conv-item:hover {
  background: var(--conv-item-hover, rgba(0, 0, 0, 0.04));
}

.conv-item-active {
  background: var(--conv-item-active, rgba(26, 115, 232, 0.12));
}

.conv-item-active:hover {
  background: var(--conv-item-active-hover, rgba(26, 115, 232, 0.18));
}

/* 重命名态下：禁用 hover 变化，提示正在编辑 */
.conv-item-renaming {
  cursor: text;
  background: var(--conv-item-renaming, rgba(26, 115, 232, 0.06));
}

.conv-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.conv-title-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.conv-pin {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 5px;
  border-radius: 3px;
  background: var(--pin-bg, #fff3cd);
  color: var(--pin-fg, #946c00);
  flex-shrink: 0;
  letter-spacing: 0.02em;
}

.conv-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary, #1a1a1a);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.conv-time {
  font-size: 11px;
  color: var(--text-secondary, #888);
  user-select: none;
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .conv-item:hover {
    --conv-item-hover: rgba(255, 255, 255, 0.06);
  }
  .conv-item-active {
    --conv-item-active: rgba(74, 144, 226, 0.22);
  }
  .conv-item-active:hover {
    --conv-item-active-hover: rgba(74, 144, 226, 0.32);
  }
  .conv-item-renaming {
    --conv-item-renaming: rgba(74, 144, 226, 0.1);
  }
  .conv-title {
    --text-primary: #f0f0f0;
  }
  .conv-time {
    --text-secondary: #888;
  }
  .conv-pin {
    --pin-bg: #5c4a00;
    --pin-fg: #ffd966;
  }
}
</style>