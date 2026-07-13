<script setup lang="ts">
// Agent 统一头像组件
//
// 职责：
//   - 在所有需要展示 Agent 头像的地方统一渲染（卡片 / 侧边栏 / 头部 / 欢迎页）
//   - 支持两种内容模式：
//       1. 有模板 → 居中渲染 Lucide 图标（深色）
//       2. 无模板 → 居中显示字母缩写（深色）
//   - 形状：8px 圆角方形（设计规范：--ip-radius-md）
//   - 配色：浅色背景 + 深色前景（bg/fg 配对），不直接用白字
//
// props:
//   - meta  AgentMeta（必传）
//   - size  头像尺寸（像素），默认 48
//
// 用法：
//   <AgentAvatar :meta="agentMeta" :size="48" />

import { computed } from "vue";
import type { AgentMeta } from "../../composables/useAgentMeta";
import { saturatedToBgFg } from "../../utils/agentAvatar";

const props = withDefaults(
  defineProps<{
    meta: AgentMeta;
    size?: number;
  }>(),
  {
    size: 48,
  },
);

/** bg/fg 配对（统一计算一次，避免重复调用 saturatedToBgFg） */
const bgFg = computed(() => saturatedToBgFg(props.meta.avatarColor));

/** 头像容器样式（动态尺寸 + 背景色） */
const containerStyle = computed<Record<string, string>>(() => ({
  width: `${props.size}px`,
  height: `${props.size}px`,
  backgroundColor: bgFg.value.bg,
}));

/** 图标 / 文字尺寸（size 的 50%） */
const innerSize = computed<number>(() => Math.max(10, Math.floor(props.size * 0.5)));

/** 文字尺寸（size 的 40%） */
const textFontSize = computed<number>(() =>
  Math.max(10, Math.floor(props.size * 0.4)),
);

/** 前景色（图标 / 文字统一用 fg，深色） */
const fgColor = computed<string>(() => bgFg.value.fg);
</script>

<template>
  <div
    class="agent-avatar"
    :style="containerStyle"
    :aria-label="`${meta.description || 'Agent'} 头像`"
    role="img"
  >
    <component
      :is="meta.icon"
      v-if="meta.icon"
      :size="innerSize"
      :color="fgColor"
      :stroke-width="2.25"
      aria-hidden="true"
    />
    <span
      v-else
      class="avatar-text"
      :style="{
        fontSize: `${textFontSize}px`,
        color: fgColor,
      }"
    >{{ meta.avatarText }}</span>
  </div>
</template>

<style scoped>
.agent-avatar {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border-radius: var(--ip-radius-md);
  overflow: hidden;
  user-select: none;
  /* 让头像在 hover / 选中态时不会出现意外描边 */
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.04);
}

.avatar-text {
  font-weight: var(--ip-font-weight-bold, 700);
  line-height: 1;
  letter-spacing: -0.01em;
  text-align: center;
}
</style>
