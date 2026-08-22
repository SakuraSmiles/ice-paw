<!-- AgentPicker — 选择 Agent 弹窗：项目成员优先、关闭条件 -->
<script setup lang="ts">
import { computed } from "vue";
import { useAgentStore } from "../../stores/agent";
import EntityAvatar from "../common/EntityAvatar.vue";

const props = defineProps<{ agentIds?: string[] }>();
const emit = defineEmits<{
  select: [agentId: string];
  close: [];
}>();

const agent = useAgentStore();

/** 传入 agentIds 时只展示这些 agent（如限项目成员） */
const visibleAgents = computed(() =>
  props.agentIds ? agent.list.filter((a) => props.agentIds!.includes(a.id)) : agent.list,
);
</script>

<template>
  <div class="picker-overlay" @click.self="emit('close')">
    <div class="picker-panel">
      <div class="picker-header">
        <h3 class="picker-title">选择助手</h3>
        <button class="picker-close" @click="emit('close')">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
      <div class="picker-list">
        <button
          v-for="a in visibleAgents"
          :key="a.id"
          class="picker-item"
          @click="emit('select', a.id)"
        >
          <!-- 无图走默认头像（链路内置于 EntityAvatar，2026-08-22 全语境统一） -->
          <EntityAvatar class="picker-avatar" :name="a.name" :image="a.avatar" size="lg" />
          <div class="picker-info">
            <div class="picker-name">{{ a.name }}</div>
            <div class="picker-desc">{{ a.description || a.model }}</div>
          </div>
          <svg class="picker-arrow" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.picker-overlay {
  position: fixed; inset: 0;
  z-index: var(--ip-z-modal-overlay);
  background: rgba(0,0,0,0.3);
  display: flex; align-items: center; justify-content: center;
}
.picker-panel {
  width: 360px; max-height: 70vh;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
  box-shadow: var(--ip-shadow-xl);
  display: flex; flex-direction: column;
  overflow: hidden;
}
.picker-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 16px 16px 12px; flex-shrink: 0;
}
.picker-title { font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); margin: 0; }
.picker-close {
  display: flex; align-items: center; justify-content: center;
  width: 28px; height: 28px; border-radius: var(--ip-radius-md); cursor: pointer;
  color: var(--ip-color-text-secondary); transition: all var(--ip-duration-fast) var(--ip-ease-out);
  background: none; border: none;
}
.picker-close:hover { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }

.picker-list {
  flex: 1; overflow-y: auto; padding: 4px 8px 12px;
  display: flex; flex-direction: column; gap: 2px;
}
.picker-item {
  display: flex; align-items: center; gap: var(--ip-spacing-3);
  width: 100%; padding: 10px 12px; text-align: left;
  border: none; border-radius: var(--ip-radius-lg); cursor: pointer;
  background: transparent; font: inherit; color: inherit;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.picker-item:hover { background: var(--ip-color-bg-sidebar-item-hover); }
/* EntityAvatar 容器（lg=36px；视觉由组件内三级链接管） */
.picker-avatar { flex-shrink: 0; }
.picker-info { flex: 1; min-width: 0; }
.picker-name { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); }
.picker-desc { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.picker-arrow { color: var(--ip-color-text-tertiary); flex-shrink: 0; }
</style>
