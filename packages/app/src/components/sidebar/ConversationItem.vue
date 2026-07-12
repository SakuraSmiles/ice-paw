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

import { Pin, MoreHorizontal } from "lucide-vue-next";
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
        <Pin
          v-if="conv.pinned"
          :size="12"
          class="conv-pin"
          :fill="active ? 'currentColor' : 'none'"
          aria-label="已置顶"
        />
        <span class="conv-title">{{ conv.title || "新会话" }}</span>
        <MoreHorizontal
          :size="14"
          class="conv-more"
          aria-hidden="true"
        />
      </div>
      <div class="conv-time">{{ formatRelative(conv.updated_at) }}</div>
    </div>
  </div>
</template>

<style scoped>
.conv-item {
  position: relative;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: var(--ip-transition-colors);
  user-select: none;
}

.conv-item:hover {
  background: var(--ip-color-bg-tertiary);
}

/* 选中态：亮色 primary-50 + primary-700；暗色 primary-900 + primary-100 */
.conv-item-active {
  background: var(--ip-primary-50);
  color: var(--ip-primary-700);
}

.conv-item-active:hover {
  background: var(--ip-primary-100);
}

/* 暗色模式：Vue scoped 编译时只会给最后一个 class 加 data-v，data-theme 祖先保持原样 */
[data-theme="dark"] .conv-item-active {
  background: var(--ip-primary-900);
  color: var(--ip-primary-100);
}

[data-theme="dark"] .conv-item-active:hover {
  background: var(--ip-primary-800);
}

/* 重命名态下：禁用 hover 变化，提示正在编辑 */
.conv-item-renaming {
  cursor: text;
  background: var(--ip-primary-50);
}

[data-theme="dark"] .conv-item-renaming {
  background: var(--ip-primary-900);
}

.conv-content {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
  min-width: 0;
}

.conv-title-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  min-width: 0;
}

.conv-pin {
  flex-shrink: 0;
  color: var(--ip-warning-text);
  /* 选中态跟随主文字色 */
  transition: color var(--ip-duration-immediate) var(--ip-ease-out);
}

.conv-item-active .conv-pin {
  color: var(--ip-primary-700);
}

[data-theme="dark"] .conv-item-active .conv-pin {
  color: var(--ip-primary-100);
}

.conv-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.conv-item-active .conv-title {
  color: var(--ip-primary-700);
}

[data-theme="dark"] .conv-item-active .conv-title {
  color: var(--ip-primary-100);
}

.conv-more {
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  opacity: 0;
  transition: opacity var(--ip-duration-immediate) var(--ip-ease-out);
}

.conv-item:hover .conv-more,
.conv-item:focus-within .conv-more {
  opacity: 1;
}

.conv-item-active .conv-more {
  color: var(--ip-primary-700);
  opacity: 0.7;
}

[data-theme="dark"] .conv-item-active .conv-more {
  color: var(--ip-primary-100);
}

.conv-time {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  user-select: none;
}

.conv-item-active .conv-time {
  color: var(--ip-primary-600);
  opacity: 0.85;
}

[data-theme="dark"] .conv-item-active .conv-time {
  color: var(--ip-primary-300);
}
</style>
