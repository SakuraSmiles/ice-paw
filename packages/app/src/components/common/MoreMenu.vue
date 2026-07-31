<script setup lang="ts">
// MoreMenu.vue — 更多操作下拉菜单（⋮ kebab）
// 通用下拉：点 ⋙ 展开菜单项，点外部收起。
// item 带 confirmText 时，点击不立即触发，而是就地切换为「确认/取消」二次确认态。
import { ref, onMounted, onUnmounted } from "vue";

defineProps<{
  items: { label: string; value: string; danger?: boolean; confirmText?: string }[];
}>();

const emit = defineEmits<{ select: [value: string] }>();

const open = ref(false);
const confirming = ref<string | null>(null);
const wrapRef = ref<HTMLElement | null>(null);

function toggle() {
  open.value = !open.value;
  if (!open.value) confirming.value = null;
}

function onItemClick(item: { value: string; confirmText?: string }) {
  if (item.confirmText) {
    confirming.value = item.value; // 进入二次确认态
  } else {
    open.value = false;
    emit("select", item.value);
  }
}
function doConfirm(value: string) {
  confirming.value = null;
  open.value = false;
  emit("select", value);
}
function cancelConfirm() {
  confirming.value = null;
}

function onDocClick(e: MouseEvent) {
  if (wrapRef.value && !wrapRef.value.contains(e.target as Node)) {
    open.value = false;
    confirming.value = null;
  }
}
// 用 capture 阶段监听：MoreMenu 常处在带 @click.stop 的容器（如 expand-panel）内，
// 冒泡阶段会被 stop 拦截导致收不到外部点击；capture 在 stop 之前触发，能正常收起。
onMounted(() => document.addEventListener("click", onDocClick, true));
onUnmounted(() => document.removeEventListener("click", onDocClick, true));
</script>

<template>
  <div ref="wrapRef" class="more-wrap">
    <button class="more-btn" :class="{ active: open }" title="更多操作" @click="toggle">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
        <circle cx="12" cy="5" r="1.6" /><circle cx="12" cy="12" r="1.6" /><circle cx="12" cy="19" r="1.6" />
      </svg>
    </button>
    <Transition name="more-drop">
      <div v-if="open" class="more-menu" @click.stop>
        <template v-for="item in items" :key="item.value">
          <!-- 二次确认态 -->
          <div v-if="confirming === item.value" class="confirm-row">
            <span class="confirm-text">{{ item.confirmText }}</span>
            <div class="confirm-actions">
              <button class="confirm-yes" title="确认删除" @click="doConfirm(item.value)">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              </button>
              <button class="confirm-no" title="取消" @click="cancelConfirm">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
              </button>
            </div>
          </div>
          <!-- 普通项 -->
          <button
            v-else
            :class="['more-item', { danger: item.danger }]"
            @click="onItemClick(item)"
          >
            {{ item.label }}
          </button>
        </template>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.more-wrap {
  position: relative;
}

.more-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  background: none;
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.more-btn:hover,
.more-btn.active {
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
}

.more-menu {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  z-index: 50;
  min-width: 148px;
  padding: 4px;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
}

.more-item {
  display: block;
  width: 100%;
  padding: 7px 10px;
  text-align: left;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: none;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.more-item:hover {
  background-color: var(--ip-color-bg-tertiary);
}
.more-item.danger {
  color: var(--ip-danger-base);
}
.more-item.danger:hover {
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}

/* 二次确认态（就地，对齐 ChatHeader 会话删除的样式） */
.confirm-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  gap: 8px;
}
.confirm-text {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  white-space: nowrap;
}
.confirm-actions {
  display: flex;
  gap: 4px;
}
.confirm-yes,
.confirm-no {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  background: transparent;
  color: var(--ip-color-text-tertiary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.confirm-yes:hover {
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}
.confirm-no:hover {
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}

/* 下拉动画 */
.more-drop-enter-active {
  animation: more-in 0.12s ease-out;
}
.more-drop-leave-active {
  animation: more-in 0.1s ease-in reverse;
}
@keyframes more-in {
  from { opacity: 0; transform: translateY(-4px) scale(0.96); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}
</style>
