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
  z-index: 999;
  min-width: 140px;
  max-width: 240px;
  padding: 4px;
  background: var(--menu-bg, #ffffff);
  border: 1px solid var(--menu-border, #d0d0d0);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.14);
  display: flex;
  flex-direction: column;
}

.ctx-item {
  appearance: none;
  border: none;
  background: transparent;
  text-align: left;
  padding: 8px 14px;
  font-family: inherit;
  font-size: 13px;
  color: var(--text-primary, #1a1a1a);
  border-radius: 4px;
  cursor: pointer;
  transition: background 80ms ease;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ctx-item:hover {
  background: var(--menu-item-hover, #f0f0f0);
}

.ctx-item-danger {
  color: var(--danger-fg, #d93025);
}
.ctx-item-danger:hover {
  background: var(--danger-bg-hover, #fde8e8);
}

/* 进出动画 */
.ctx-menu-enter-from {
  opacity: 0;
  transform: scale(0.96);
}
.ctx-menu-enter-active {
  transition: opacity 120ms ease, transform 120ms ease;
}
.ctx-menu-leave-to {
  opacity: 0;
  transform: scale(0.96);
}
.ctx-menu-leave-active {
  transition: opacity 120ms ease, transform 120ms ease;
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .context-menu {
    --menu-bg: #2a2a3a;
    --menu-border: #3a3a4a;
  }
  .ctx-item {
    --text-primary: #f0f0f0;
  }
  .ctx-item:hover {
    --menu-item-hover: #3a3a4a;
  }
  .ctx-item-danger {
    --danger-fg: #ff6b6b;
  }
  .ctx-item-danger:hover {
    --danger-bg-hover: #3a2020;
  }
}
</style>