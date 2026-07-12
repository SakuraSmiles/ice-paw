<script setup lang="ts">
// Agent 选择器
//
// 职责：
//   - 顶部横条展示当前 Agent 名称 + 下拉箭头
//   - 点击展开下拉列表（absolute 定位）
//   - 点击列表项切换 Agent
//   - 底部固定项「管理 Agent →」跳转路由
//
// 数据源：agentsStore（无需 props）

import { ref, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { useAgentsStore } from "../../stores/agents";

const agentsStore = useAgentsStore();
const router = useRouter();

/** 下拉是否展开 */
const expanded = ref<boolean>(false);

/** 切换展开 */
function toggle(): void {
  expanded.value = !expanded.value;
}

/** 选中 Agent */
function pick(id: string): void {
  agentsStore.setCurrent(id);
  expanded.value = false;
}

/** 跳转到管理页 */
function goToManager(): void {
  expanded.value = false;
  void router.push({ name: "AgentManager" });
}

/** 点击外部关闭 */
function onDocumentClick(e: MouseEvent): void {
  if (!expanded.value) return;
  const target = e.target as HTMLElement | null;
  if (target && target.closest(".agent-selector")) return;
  expanded.value = false;
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
});

onUnmounted(() => {
  document.removeEventListener("click", onDocumentClick);
});
</script>

<template>
  <div class="agent-selector">
    <button class="selector-trigger" type="button" @click.stop="toggle">
      <span class="selector-label">Agent</span>
      <span class="selector-name">{{ agentsStore.current?.name ?? "未选择" }}</span>
      <span :class="['selector-arrow', expanded ? 'arrow-up' : 'arrow-down']">v</span>
    </button>

    <Transition name="dropdown">
      <div v-if="expanded" class="selector-dropdown" @click.stop>
        <div v-if="agentsStore.agents.length === 0" class="dropdown-empty">暂无 Agent</div>
        <template v-else>
          <button
            v-for="agent in agentsStore.agents"
            :key="agent.id"
            :class="[
              'dropdown-item',
              agentsStore.currentId === agent.id ? 'dropdown-item-active' : '',
            ]"
            type="button"
            @click="pick(agent.id)"
          >
            <span class="item-name">{{ agent.name }}</span>
            <span class="item-meta">{{ agent.provider }} · {{ agent.model }}</span>
          </button>
        </template>
        <div class="dropdown-divider" />
        <button class="dropdown-item dropdown-manage" type="button" @click="goToManager">
          管理 Agent →
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.agent-selector {
  position: relative;
  padding: 12px 14px;
  border-bottom: 1px solid var(--ip-color-border-default);
}

.selector-trigger {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  transition: background-color var(--ip-duration-immediate) var(--ip-ease-out), border-color var(--ip-duration-immediate) var(--ip-ease-out);
}

.selector-trigger:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}

.selector-label {
  font-size: 10px;
  font-weight: var(--ip-font-weight-semibold);
  text-transform: uppercase;
  color: var(--ip-color-text-tertiary);
  letter-spacing: 0.05em;
  flex-shrink: 0;
}

.selector-name {
  flex: 1;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.selector-arrow {
  font-size: 10px;
  color: var(--ip-color-text-tertiary);
  flex-shrink: 0;
  font-family: var(--ip-font-mono);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}

.arrow-up {
  transform: rotate(180deg);
}

.selector-dropdown {
  position: absolute;
  top: calc(100% - 1px);
  left: 14px;
  right: 14px;
  z-index: var(--ip-z-dropdown);
  max-height: 320px;
  overflow-y: auto;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  padding: var(--ip-spacing-1);
  display: flex;
  flex-direction: column;
}

.dropdown-empty {
  padding: var(--ip-spacing-4);
  text-align: center;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
}

.dropdown-item {
  appearance: none;
  border: none;
  background: transparent;
  text-align: left;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  display: flex;
  flex-direction: column;
  gap: 2px;
  transition: background-color var(--ip-duration-immediate) var(--ip-ease-out);
}

.dropdown-item:hover {
  background: var(--ip-color-bg-tertiary);
}

.dropdown-item-active {
  background: var(--ip-primary-50);
  color: var(--ip-color-text-link);
}

.dropdown-item-active:hover {
  background: var(--ip-primary-100);
}

.item-name {
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-meta {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dropdown-divider {
  height: 1px;
  margin: var(--ip-spacing-1) 6px;
  background: var(--ip-color-border-default);
}

.dropdown-manage {
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-link);
}

/* 下拉动画 */
.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
.dropdown-enter-active,
.dropdown-leave-active {
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out), transform var(--ip-duration-fast) var(--ip-ease-out);
}
</style>