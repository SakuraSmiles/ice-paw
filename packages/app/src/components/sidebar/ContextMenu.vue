<script setup lang="ts">
// 全局右键菜单浮层
//
// 职责：
//   - 渲染由 useContextMenu 单例提供的菜单项
//   - 固定定位（Teleport to body）在视口指定坐标
//   - 点击项执行 handler 后自动关闭
//   - 点击菜单外部 / 按 Esc 关闭
//
// props: 无（数据来自 useContextMenu 单例）
// emits: 无（关闭通过 composable.closeMenu() 触发）

import { onMounted, onUnmounted } from "vue";
import { useContextMenu } from "../../composables/useContextMenu";

const ctx = useContextMenu();

/**
 * 点击菜单项的统一处理：
 *   先关闭菜单，再触发 handler。
 *   顺序很重要：handler 若要再次打开菜单（例如删除后刷新触发新一轮），
 *   必须避免被立刻关掉。
 */
function onItemClick(handler: () => void): void {
  ctx.closeMenu();
  try {
    handler();
  } catch {
    // 业务错误由 handler 内部自行处理（Toast 等），此处不阻断
  }
}

/** document 点击：点击菜单外部时关闭 */
function onDocumentClick(e: MouseEvent): void {
  if (!ctx.state.visible) return;
  const target = e.target as HTMLElement | null;
  if (target && target.closest(".context-menu")) return;
  ctx.closeMenu();
}

/** Esc 关闭菜单 */
function onKeydown(e: KeyboardEvent): void {
  if (ctx.state.visible && e.key === "Escape") {
    ctx.closeMenu();
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
  document.addEventListener("keydown", onKeydown);
});

onUnmounted(() => {
  document.removeEventListener("click", onDocumentClick);
  document.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <Teleport to="body">
    <Transition name="ctx-menu">
      <div
        v-if="ctx.state.visible"
        class="context-menu"
        :style="{ left: ctx.state.position.x + 'px', top: ctx.state.position.y + 'px' }"
        role="menu"
        @click.stop
        @contextmenu.stop.prevent
      >
        <button
          v-for="(item, idx) in ctx.state.items"
          :key="idx"
          :class="['ctx-item', item.danger ? 'ctx-item-danger' : '']"
          role="menuitem"
          type="button"
          @click="onItemClick(item.handler)"
        >
          {{ item.label }}
        </button>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.context-menu {
  /* 固定定位，由内联 style 写入 left/top */
  position: fixed;
  z-index: var(--ip-z-popover);
  min-width: 140px;
  max-width: 240px;
  padding: var(--ip-spacing-1);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  display: flex;
  flex-direction: column;
}

.ctx-item {
  appearance: none;
  border: none;
  background: transparent;
  text-align: left;
  padding: var(--ip-spacing-2) 14px;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition: background-color var(--ip-duration-immediate) var(--ip-ease-out);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ctx-item:hover {
  background: var(--ip-color-bg-tertiary);
}

.ctx-item-danger {
  color: var(--ip-danger-base);
}
.ctx-item-danger:hover {
  background: var(--ip-danger-bg);
}

/* 进出动画 */
.ctx-menu-enter-from,
.ctx-menu-leave-to {
  opacity: 0;
  transform: scale(0.96);
}
.ctx-menu-enter-active,
.ctx-menu-leave-active {
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out), transform var(--ip-duration-fast) var(--ip-ease-out);
}
</style>