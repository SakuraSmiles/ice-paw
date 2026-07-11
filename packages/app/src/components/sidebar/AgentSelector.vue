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
  border-bottom: 1px solid var(--border, #e0e0e0);
}

.selector-trigger {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  background: var(--selector-bg, #f5f5f5);
  border: 1px solid var(--selector-border, #e0e0e0);
  border-radius: 6px;
  cursor: pointer;
  font-family: inherit;
  font-size: 13px;
  transition: background 80ms ease, border-color 80ms ease;
}

.selector-trigger:hover {
  background: var(--selector-bg-hover, #ebebeb);
  border-color: var(--selector-border-hover, #c0c0c0);
}

.selector-label {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-secondary, #888);
  letter-spacing: 0.05em;
  flex-shrink: 0;
}

.selector-name {
  flex: 1;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.selector-arrow {
  font-size: 10px;
  color: var(--text-secondary, #888);
  flex-shrink: 0;
  font-family: monospace;
  transition: transform 120ms ease;
}

.arrow-up {
  transform: rotate(180deg);
}

.selector-dropdown {
  position: absolute;
  top: calc(100% - 1px);
  left: 14px;
  right: 14px;
  z-index: 50;
  max-height: 320px;
  overflow-y: auto;
  background: var(--menu-bg, #ffffff);
  border: 1px solid var(--menu-border, #d0d0d0);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  padding: 4px;
  display: flex;
  flex-direction: column;
}

.dropdown-empty {
  padding: 16px;
  text-align: center;
  font-size: 13px;
  color: var(--text-secondary, #888);
}

.dropdown-item {
  appearance: none;
  border: none;
  background: transparent;
  text-align: left;
  padding: 8px 12px;
  border-radius: 4px;
  cursor: pointer;
  font-family: inherit;
  font-size: 13px;
  color: var(--text-primary, #1a1a1a);
  display: flex;
  flex-direction: column;
  gap: 2px;
  transition: background 80ms ease;
}

.dropdown-item:hover {
  background: var(--menu-item-hover, #f0f0f0);
}

.dropdown-item-active {
  background: var(--menu-item-active, rgba(26, 115, 232, 0.1));
  color: var(--accent-fg, #1a73e8);
}

.dropdown-item-active:hover {
  background: var(--menu-item-active-hover, rgba(26, 115, 232, 0.16));
}

.item-name {
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-meta {
  font-size: 11px;
  color: var(--text-secondary, #888);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dropdown-divider {
  height: 1px;
  margin: 4px 6px;
  background: var(--divider, #e0e0e0);
}

.dropdown-manage {
  font-weight: 500;
  color: var(--accent-fg, #1a73e8);
}

/* 下拉动画 */
.dropdown-enter-from {
  opacity: 0;
  transform: translateY(-4px);
}
.dropdown-enter-active {
  transition: opacity 120ms ease, transform 120ms ease;
}
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
.dropdown-leave-active {
  transition: opacity 120ms ease, transform 120ms ease;
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .agent-selector {
    --border: #3a3a4a;
  }
  .selector-trigger {
    --selector-bg: #2a2a3a;
    --selector-border: #3a3a4a;
    --selector-bg-hover: #353548;
    --selector-border-hover: #4a4a5a;
  }
  .selector-label,
  .selector-arrow,
  .dropdown-empty,
  .item-meta {
    --text-secondary: #888;
  }
  .selector-name,
  .dropdown-item {
    --text-primary: #f0f0f0;
  }
  .selector-dropdown {
    --menu-bg: #2a2a3a;
    --menu-border: #3a3a4a;
  }
  .dropdown-item:hover {
    --menu-item-hover: #3a3a4a;
  }
  .dropdown-item-active {
    --menu-item-active: rgba(74, 144, 226, 0.2);
    --accent-fg: #6ba9e8;
  }
  .dropdown-item-active:hover {
    --menu-item-active-hover: rgba(74, 144, 226, 0.32);
  }
  .dropdown-divider {
    --divider: #3a3a4a;
  }
  .dropdown-manage {
    --accent-fg: #6ba9e8;
  }
}
</style>