<script setup lang="ts">
/**
 * IpToolDrawer — IcePaw 工具抽屉
 *
 * 规范：icepaw-tool-drawer-specs.md §二
 * a11y：role="region" + role="tablist/tab/tabpanel"
 *
 * 微交互要点：
 *  - 展开：max-height 0→280px (250ms) + opacity fade (80ms)
 *  - 折叠：reverse animation (200ms + 50ms delay)
 *  - Tab 下划线：150ms ease-out slide
 *  - hover：按钮边框变色 + 文字微亮
 */
import { computed, ref, watch, nextTick, useId } from 'vue'
import { Plus, X } from 'lucide-vue-next'
import type { ToolDrawerProps, ToolDrawerEmits } from './types'

const props = withDefaults(defineProps<ToolDrawerProps>(), {
  toggleLabel: '+ Tools',
  closeLabel: '收起工具面板',
  maxHeight: 280,
  disabled: false,
})

const emit = defineEmits<ToolDrawerEmits>()

const internalId = useId()
const drawerId = computed(() => `ip-tool-drawer-${internalId}`)

const animating = ref(false)
const bodyVisible = ref(false)

/* ----- active Tab ----- */
const activeTabId = ref<string>(props.activeTab ?? props.tabs[0]?.id ?? '')

watch(
  () => props.activeTab,
  (v) => {
    if (v !== undefined && v !== null) activeTabId.value = v
  },
)
watch(
  () => props.tabs,
  (tabs) => {
    if (!activeTabId.value && tabs.length > 0) {
      activeTabId.value = tabs[0]!.id
    }
  },
  { immediate: true },
)

function selectTab(tabId: string, index: number): void {
  activeTabId.value = tabId
  emit('tabChange', tabId)
  // 聚焦新 Tab
  void nextTick(() => {
    const el = document.getElementById(`${drawerId.value}-tab-${index}`)
    el?.focus()
  })
}

/* ----- 展开 / 折叠 ----- */
function toggle(): void {
  if (props.disabled) return
  if (animating.value) return
  const nextOpen = !props.open
  emit('update:open', nextOpen)
  if (nextOpen) openDrawer()
  else closeDrawer()
}

function openDrawer(): void {
  animating.value = true
  bodyVisible.value = false
  // 延迟显示内容 (80ms 后)
  window.setTimeout(() => {
    bodyVisible.value = true
  }, 80)
  // 动画完成后
  window.setTimeout(() => {
    animating.value = false
    emit('expanded')
  }, 280)
}

function closeDrawer(): void {
  animating.value = true
  bodyVisible.value = false
  window.setTimeout(() => {
    animating.value = false
    emit('collapsed')
  }, 220)
}

function closeViaButton(): void {
  if (props.disabled || animating.value) return
  emit('update:open', false)
  closeDrawer()
}

/* ----- 键盘导航: Tab 栏 ----- */
function onTabKeydown(e: KeyboardEvent, idx: number): void {
  const tabs = props.tabs
  if (tabs.length === 0) return

  let nextIdx = idx
  switch (e.key) {
    case 'ArrowRight':
    case 'ArrowDown':
      e.preventDefault()
      nextIdx = (idx + 1) % tabs.length
      break
    case 'ArrowLeft':
    case 'ArrowUp':
      e.preventDefault()
      nextIdx = (idx - 1 + tabs.length) % tabs.length
      break
    case 'Home':
      e.preventDefault()
      nextIdx = 0
      break
    case 'End':
      e.preventDefault()
      nextIdx = tabs.length - 1
      break
    case 'Enter':
    case ' ':
      e.preventDefault()
      selectTab(tabs[idx]!.id, idx)
      return
    case 'Escape':
      e.preventDefault()
      closeViaButton()
      return
    default:
      return
  }
  selectTab(tabs[nextIdx]!.id, nextIdx)
}

/* ----- max-height CSS var ----- */
const maxHeightPx = computed<string>(() =>
  typeof props.maxHeight === 'number' ? `${props.maxHeight}px` : props.maxHeight,
)

/* ----- 当前激活 tab 在 tabs 中的 index（用于面板 aria-labelledby） ----- */
const activeTabIndex = computed<number>(() => {
  const idx = props.tabs.findIndex((t) => t.id === activeTabId.value)
  return idx < 0 ? 0 : idx
})

/* ----- 暴露方法：程序化切换 ----- */
defineExpose({ toggle, close: closeViaButton })
</script>

<template>
  <div
    :class="[
      'ip-tool-drawer',
      {
        'ip-tool-drawer--open': open,
        'ip-tool-drawer--disabled': disabled,
        'ip-tool-drawer--animating': animating,
      },
    ]"
    :style="{ '--drawer-max-height': maxHeightPx }"
  >
    <!-- ===== 折叠态: toggle 按钮 ===== -->
    <button
      v-if="!open"
      type="button"
      class="ip-tool-drawer__toggle"
      aria-expanded="false"
      :aria-controls="drawerId"
      :aria-label="`展开${toggleLabel}`"
      :disabled="disabled"
      @click="toggle"
    >
      <slot name="toggle">
        <Plus :size="14" class="ip-tool-drawer__toggle-icon" aria-hidden="true" />
        <span class="ip-tool-drawer__toggle-label">{{ toggleLabel }}</span>
      </slot>
    </button>

    <!-- ===== 展开态: 容器 ===== -->
    <Transition name="ip-tool-drawer">
      <div
        v-if="open"
        :id="drawerId"
        class="ip-tool-drawer__panel"
        role="region"
        :aria-label="ariaLabel ?? '工具面板'"
      >
        <!-- 头部栏 -->
        <div class="ip-tool-drawer__header">
          <slot name="header" :tabs="tabs" :activeTab="activeTabId">
            <!-- 关闭按钮 -->
            <button
              type="button"
              class="ip-tool-drawer__close"
              :aria-label="closeLabel"
              @click="closeViaButton"
            >
              <X :size="14" aria-hidden="true" />
            </button>

            <!-- Tab 栏 -->
            <div
              v-if="tabs.length > 0"
              class="ip-tool-drawer__tabs"
              role="tablist"
              aria-label="工具分类"
            >
              <button
                v-for="(tab, idx) in tabs"
                :key="tab.id"
                :id="`${drawerId}-tab-${idx}`"
                :class="[
                  'ip-tool-drawer__tab',
                  { 'ip-tool-drawer__tab--active': activeTabId === tab.id },
                ]"
                role="tab"
                :aria-selected="activeTabId === tab.id"
                :aria-controls="`${drawerId}-panel-${tab.id}`"
                :tabindex="activeTabId === tab.id ? 0 : -1"
                @click="selectTab(tab.id, idx)"
                @keydown="onTabKeydown($event, idx)"
              >
                {{ tab.label }}
                <span
                  v-if="activeTabId === tab.id"
                  class="ip-tool-drawer__tab-underline"
                  aria-hidden="true"
                />
              </button>
            </div>
          </slot>
        </div>

        <!-- 内容区 -->
        <div class="ip-tool-drawer__body">
          <Transition name="ip-tool-drawer__body">
            <div
              v-if="bodyVisible"
              :key="activeTabId"
              :id="`${drawerId}-panel-${activeTabId}`"
              class="ip-tool-drawer__body-inner"
              role="tabpanel"
              :aria-labelledby="`${drawerId}-tab-${activeTabIndex}`"
              tabindex="0"
            >
              <!-- 按 Tab id 匹配 slot -->
              <template v-for="tab in tabs" :key="tab.id">
                <slot
                  v-if="activeTabId === tab.id"
                  :name="`tab-${tab.id}`"
                  :active="true"
                  :tab="tab"
                />
              </template>

              <!-- 兜底 slot -->
              <slot v-if="$slots.body" name="body" :activeTab="activeTabId" />

              <!-- 完全无内容时显示空状态 -->
              <div
                v-if="
                  !$slots.body &&
                  tabs.every((t) => !$slots[`tab-${t.id}`])
                "
                class="ip-tool-drawer__empty"
              >
                未提供内容
              </div>
            </div>
          </Transition>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
/* ============================================================
 * IpToolDrawer — 视觉实现
 * 规范：icepaw-tool-drawer-specs.md §2.4-2.5
 * ============================================================ */

/* ----- 根容器 ----- */
.ip-tool-drawer {
  width: 100%;
  flex-shrink: 0;
}

/* ----- 折叠态 toggle 按钮 ----- */
.ip-tool-drawer__toggle {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  height: var(--ip-tool-drawer-toggle-h, 28px);
  padding: 0 var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  font-family: inherit;
  line-height: 1;
  color: var(--ip-color-text-tertiary);
  background: transparent;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.ip-tool-drawer__toggle:hover:not(:disabled) {
  color: var(--ip-color-text-secondary);
  border-color: var(--ip-color-border-strong);
}

.ip-tool-drawer__toggle:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.ip-tool-drawer__toggle:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.ip-tool-drawer__toggle-icon {
  flex-shrink: 0;
}

.ip-tool-drawer__toggle-label {
  line-height: 1;
}

/* ----- 展开态面板容器 ----- */
.ip-tool-drawer__panel {
  width: 100%;
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-top: none;
  border-radius: 0 0 var(--ip-radius-lg) var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-sm);
  overflow: hidden;
}

/* ----- 头部栏 ----- */
.ip-tool-drawer__header {
  display: flex;
  align-items: center;
  height: 32px;
  padding: 0 var(--ip-spacing-3);
  border-bottom: 1px solid var(--ip-color-border-default);
}

/* 关闭按钮 */
.ip-tool-drawer__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  margin-right: var(--ip-spacing-2);
  color: var(--ip-color-text-tertiary);
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  flex-shrink: 0;
  transition: var(--ip-transition-colors);
}

.ip-tool-drawer__close:hover {
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
}

.ip-tool-drawer__close:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ----- Tab 栏 ----- */
.ip-tool-drawer__tabs {
  display: flex;
  align-items: stretch;
  height: 100%;
  gap: 0;
}

.ip-tool-drawer__tab {
  position: relative;
  display: inline-flex;
  align-items: center;
  height: 100%;
  padding: 0 var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  font-family: inherit;
  color: var(--ip-color-text-tertiary);
  background: transparent;
  border: none;
  border-radius: 0;
  cursor: pointer;
  line-height: 1;
  transition: var(--ip-transition-colors);
}

.ip-tool-drawer__tab:hover {
  color: var(--ip-color-text-secondary);
}

.ip-tool-drawer__tab--active {
  color: var(--ip-primary-600);
}

.ip-tool-drawer__tab:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* Tab 下划线 */
.ip-tool-drawer__tab-underline {
  position: absolute;
  bottom: 0;
  left: var(--ip-spacing-3);
  right: var(--ip-spacing-3);
  height: 2px;
  background: var(--ip-primary-600);
  border-radius: 1px 1px 0 0;
  animation: ip-tool-drawer-underline-in var(--ip-duration-base) var(--ip-ease-out);
}

/* ----- 内容区 ----- */
.ip-tool-drawer__body {
  background: var(--ip-color-bg-primary);
  max-height: var(--drawer-max-height, 280px);
  overflow: hidden;
}

.ip-tool-drawer__body-inner {
  max-height: var(--drawer-max-height, 280px);
  overflow-y: auto;
  padding: var(--ip-spacing-3);
}

/* 滚动条 */
.ip-tool-drawer__body-inner::-webkit-scrollbar {
  width: 6px;
}
.ip-tool-drawer__body-inner::-webkit-scrollbar-track {
  background: transparent;
}
.ip-tool-drawer__body-inner::-webkit-scrollbar-thumb {
  background: var(--ip-color-bg-tertiary);
  border-radius: 3px;
}

/* 空状态 */
.ip-tool-drawer__empty {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 60px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  padding: var(--ip-spacing-4);
}

/* ----- 展开/折叠过渡 ----- */
.ip-tool-drawer-enter-active {
  transition:
    max-height var(--ip-duration-panel) var(--ip-ease-emphasized),
    opacity    var(--ip-duration-fast) var(--ip-ease-out);
  overflow: hidden;
}

.ip-tool-drawer-enter-from {
  max-height: 0;
  opacity: 0;
}

.ip-tool-drawer-enter-to {
  max-height: var(--drawer-max-height, 280px);
  opacity: 1;
}

.ip-tool-drawer-leave-active {
  transition:
    max-height var(--ip-duration-message) var(--ip-ease-in),
    opacity    var(--ip-duration-fast) var(--ip-ease-in);
  overflow: hidden;
}

.ip-tool-drawer-leave-from {
  max-height: var(--drawer-max-height, 280px);
  opacity: 1;
}

.ip-tool-drawer-leave-to {
  max-height: 0;
  opacity: 0;
}

/* ----- 内容 fade ----- */
.ip-tool-drawer__body-enter-active {
  transition: opacity var(--ip-duration-base) var(--ip-ease-out);
  transition-delay: 80ms;
}
.ip-tool-drawer__body-enter-from {
  opacity: 0;
}
.ip-tool-drawer__body-enter-to {
  opacity: 1;
}

.ip-tool-drawer__body-leave-active {
  transition: opacity var(--ip-duration-fast) var(--ip-ease-in);
}
.ip-tool-drawer__body-leave-from {
  opacity: 1;
}
.ip-tool-drawer__body-leave-to {
  opacity: 0;
}

/* ----- 响应式: 窄屏 modal ----- */
@media (max-width: 767px) {
  .ip-tool-drawer__panel {
    position: fixed;
    bottom: 0;
    left: 0;
    right: 0;
    z-index: var(--ip-z-modal-overlay);
    max-height: 60vh;
    border-radius: var(--ip-radius-xl) var(--ip-radius-xl) 0 0;
    box-shadow: var(--ip-shadow-xl);
    border: none;
  }
}
</style>
