<script setup lang="ts">
// Agent 选择器
//
// 职责：
//   - 顶部横条展示当前 Agent 头像 + 名称 + 下拉箭头
//   - 点击展开下拉列表（absolute 定位）
//   - 下拉项显示头像 + 名称 + 描述
//   - 点击列表项切换 Agent
//   - 底部固定项「管理 Agent →」跳转路由
//
// 数据源：agentsStore + useAgentMeta

import { computed, ref, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { ChevronDown, ArrowRight } from "lucide-vue-next";
import { useAgentsStore } from "../../stores/agents";
import { useAgentMeta, type AgentMeta } from "../../composables/useAgentMeta";
import AgentAvatar from "../common/AgentAvatar.vue";

const agentsStore = useAgentsStore();
const router = useRouter();
const agentMeta = useAgentMeta();

/** 下拉是否展开 */
const expanded = ref<boolean>(false);

/**
 * 获取 Agent 的元数据缓存。
 * 用 Map 避免在模板中反复调用 getFullMeta（每次返回新对象会触发不必要的更新）。
 */
const metaCache = computed(() => {
  const map = new Map<string, AgentMeta | null>();
  for (const agent of agentsStore.agents) {
    map.set(agent.id, agentMeta.getFullMeta(agent));
  }
  return map;
});

/** 当前 Agent 的 meta */
const currentMeta = computed<AgentMeta | null>(() => {
  if (!agentsStore.currentId) return null;
  return metaCache.value.get(agentsStore.currentId) ?? null;
});

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
      <!-- trigger 小头像（24px） -->
      <AgentAvatar
        v-if="currentMeta"
        :meta="currentMeta"
        :size="24"
      />
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
            <div class="item-row">
              <AgentAvatar
                v-if="metaCache.get(agent.id)"
                :meta="metaCache.get(agent.id)!"
                :size="24"
              />
              <div v-else class="avatar-placeholder" :style="{ width: '24px', height: '24px' }" />
              <div class="item-text">
                <span class="item-name">{{ agent.name }}</span>
                <span
                  v-if="metaCache.get(agent.id)?.description"
                  class="item-desc"
                >{{ metaCache.get(agent.id)!.description }}</span>
              </div>
            </div>
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

/* 选中态 */
.dropdown-item-active {
  background: var(--ip-primary-50);
  color: var(--ip-primary-700);
}

.dropdown-item-active:hover {
  background: var(--ip-primary-100);
}

[data-theme="dark"] .dropdown-item-active {
  background: var(--ip-primary-900);
  color: var(--ip-primary-100);
}

[data-theme="dark"] .dropdown-item-active:hover {
  background: var(--ip-primary-800);
}

/* 下拉项内部布局：头像 + 文字 */
.item-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}

.item-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1;
}

.item-name {
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-desc {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dropdown-item-active .item-desc {
  color: var(--ip-primary-600);
}

[data-theme="dark"] .dropdown-item-active .item-desc {
  color: var(--ip-primary-300);
}

.dropdown-divider {
  height: 1px;
  margin: var(--ip-spacing-1) var(--ip-spacing-2);
  background: var(--ip-color-border-default);
}

.dropdown-manage {
  display: flex;
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
