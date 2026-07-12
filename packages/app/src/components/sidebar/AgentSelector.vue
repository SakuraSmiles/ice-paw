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
import { ChevronDown, ArrowRight } from "lucide-vue-next";
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
    <button
      :class="['selector-trigger', { 'selector-trigger-open': expanded }]"
      type="button"
      :aria-expanded="expanded"
      aria-haspopup="listbox"
      @click.stop="toggle"
    >
      <span class="selector-label">Agent</span>
      <span class="selector-name">{{ agentsStore.current?.name ?? "未选择" }}</span>
      <ChevronDown
        :size="14"
        :class="['selector-arrow', { 'selector-arrow-open': expanded }]"
        aria-hidden="true"
      />
    </button>

    <Transition name="dropdown">
      <div v-if="expanded" class="selector-dropdown" role="listbox" @click.stop>
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
            role="option"
            :aria-selected="agentsStore.currentId === agent.id"
            @click="pick(agent.id)"
          >
            <span class="item-name">{{ agent.name }}</span>
            <span class="item-meta">{{ agent.provider }} · {{ agent.model }}</span>
          </button>
        </template>
        <div class="dropdown-divider" />
        <button class="dropdown-item dropdown-manage" type="button" @click="goToManager">
          <span>管理 Agent</span>
          <ArrowRight :size="14" aria-hidden="true" />
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.agent-selector {
  position: relative;
  padding: var(--ip-spacing-3);
  border-bottom: 1px solid var(--ip-color-border-default);
}

.selector-trigger {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  transition: var(--ip-transition-colors);
}

.selector-trigger:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}

.selector-trigger:focus-visible {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.selector-trigger-open {
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
  min-width: 0;
}

.selector-arrow {
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out), color var(--ip-duration-immediate) var(--ip-ease-out);
}

.selector-arrow-open {
  transform: rotate(180deg);
  color: var(--ip-color-text-secondary);
}

.selector-dropdown {
  position: absolute;
  top: calc(100% - 1px);
  left: var(--ip-spacing-3);
  right: var(--ip-spacing-3);
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
  transition: var(--ip-transition-colors);
}

.dropdown-item:hover {
  background: var(--ip-color-bg-tertiary);
}

.dropdown-item:focus-visible {
  outline: none;
  background: var(--ip-color-bg-tertiary);
  box-shadow: var(--ip-shadow-focus);
}

/* 选中态：亮色 primary-50 + primary-700；暗色 primary-900 + primary-100 */
.dropdown-item-active {
  background: var(--ip-primary-50);
  color: var(--ip-primary-700);
}

.dropdown-item-active:hover {
  background: var(--ip-primary-100);
}

/* 暗色模式：祖先选择器 [data-theme="dark"] 不被 scope，class 选择器自动加上 data-v */
[data-theme="dark"] .dropdown-item-active {
  background: var(--ip-primary-900);
  color: var(--ip-primary-100);
}

[data-theme="dark"] .dropdown-item-active:hover {
  background: var(--ip-primary-800);
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

.dropdown-item-active .item-meta {
  color: var(--ip-primary-600);
}

[data-theme="dark"] .dropdown-item-active .item-meta {
  color: var(--ip-primary-300);
}

.dropdown-divider {
  height: 1px;
  margin: var(--ip-spacing-1) var(--ip-spacing-2);
  background: var(--ip-color-border-default);
}

.dropdown-manage {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-link);
}

.dropdown-manage:hover {
  background: var(--ip-color-bg-tertiary);
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
  transition:
    opacity var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out);
}
</style>
