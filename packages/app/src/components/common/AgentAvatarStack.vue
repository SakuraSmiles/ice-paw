<script setup lang="ts">
// Agent 头像栈组件
//
// 职责：
//   - 在 ProjectCard 等场景下展示项目下的 Agent 团队缩略（最多 4 个 + N more）
//   - 重叠布局 + border 2px white 区分
//   - hover 时整体微移 + 单个头像错落浮起（stagger 50ms）
//
// props:
//   - agents  已限制为前 4 个的 Agent 列表（最多 4）
//   - total   真实总数（含未显示的）— 用于「第 4 个替换为 +N」逻辑
//   - size    单个头像尺寸（px），默认 24
//
// 设计要点：
//   - 复用 AgentAvatar.vue（背景色 + 字缩写 / Lucide 图标）
//   - 当 total > 4 时，渲染前 3 个真头像 + 第 4 个「+N」灰底 chip
//   - 当 total <= 4 时，渲染全部头像

import { computed } from "vue";
import type { Agent } from "../../types";
import { useAgentMeta } from "../../composables/useAgentMeta";
import type { AgentMeta } from "../../composables/useAgentMeta";
import AgentAvatar from "./AgentAvatar.vue";

const props = withDefaults(
  defineProps<{
    /** 已限制为前 4 个的 Agent 列表 */
    agents: Agent[];
    /** 真实总数（含未显示的） */
    total: number;
    /** 单个头像尺寸（px），默认 24 */
    size?: number;
  }>(),
  {
    size: 24,
  },
);

const agentMeta = useAgentMeta();

/** 渲染槽位：每个槽位要么是 Agent，要么是 "+N" chip */
type StackSlot =
  | { kind: "agent"; agent: Agent; meta: AgentMeta }
  | { kind: "more"; count: number };

/** 计算最终展示的槽位（最多 4 项，最后一项可能是 +N chip） */
const slots = computed<StackSlot[]>(() => {
  const result: StackSlot[] = [];
  const overflow = Math.max(0, props.total - props.agents.length);

  // 前 N-1 个真头像（如果 overflow > 0，则只取前 3 个真头像）
  const realCount = overflow > 0 ? Math.min(3, props.agents.length) : props.agents.length;

  for (let i = 0; i < realCount; i++) {
    const agent = props.agents[i];
    if (!agent) continue;
    const meta = agentMeta.getMeta(agent);
    result.push({ kind: "agent", agent, meta });
  }

  // 溢出 chip
  if (overflow > 0) {
    result.push({ kind: "more", count: overflow + realCount === 4 ? overflow + (props.agents.length - realCount) : props.total - realCount });
  }

  return result;
});

/** 单个头像槽位的尺寸样式（动态 size + border 2px） */
function avatarStyle(): Record<string, string> {
  return {
    width: `${props.size}px`,
    height: `${props.size}px`,
  };
}
</script>

<template>
  <div
    class="avatar-stack"
    :aria-label="`${total} 个 Agent`"
    role="img"
  >
    <template v-for="(slot, idx) in slots" :key="idx">
      <!-- 真头像 -->
      <div v-if="slot.kind === 'agent'" class="avatar-stack__item" :style="avatarStyle()">
        <AgentAvatar :meta="slot.meta" :size="props.size" />
      </div>
      <!-- +N chip -->
      <span
        v-else
        class="avatar-stack__item avatar-stack__more"
        :style="{ width: `${props.size}px`, height: `${props.size}px`, fontSize: `${Math.max(10, Math.floor(props.size * 0.5))}px` }"
      >+{{ slot.count }}</span>
    </template>
  </div>
</template>

<style scoped>
.avatar-stack {
  display: inline-flex;
  align-items: center;
  transition: transform var(--ip-duration-base) var(--ip-ease-out);
}

.avatar-stack:hover {
  transform: translateX(-2px);
}

.avatar-stack__item {
  margin-left: -6px;
  border: 2px solid var(--ip-white);
  border-radius: var(--ip-radius-full);
  transition: transform var(--ip-duration-base) var(--ip-ease-out);
  flex-shrink: 0;
}

.avatar-stack__item:first-child {
  margin-left: 0;
}

/* hover 错落浮起（每个头像 50ms stagger） */
.avatar-stack:hover .avatar-stack__item:nth-child(1) {
  transform: translateY(-1px);
  transition-delay: 0ms;
}
.avatar-stack:hover .avatar-stack__item:nth-child(2) {
  transform: translateY(-2px);
  transition-delay: 50ms;
}
.avatar-stack:hover .avatar-stack__item:nth-child(3) {
  transform: translateY(-1px);
  transition-delay: 100ms;
}
.avatar-stack:hover .avatar-stack__item:nth-child(4) {
  transform: translateY(-2px);
  transition-delay: 150ms;
}

.avatar-stack__more {
  background: var(--ip-gray-100);
  color: var(--ip-color-text-secondary);
  font-family: var(--ip-font-mono);
  font-weight: 600;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  user-select: none;
}
</style>