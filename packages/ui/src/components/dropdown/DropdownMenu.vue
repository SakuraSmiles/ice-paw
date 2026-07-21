<script setup lang="ts">
/**
 * DropdownMenu — IcePaw 溢出下拉菜单（受控）
 *
 * 规范：icepaw-p0-component-specs.md §五
 * 微交互：
 *  - 浮层 enter：opacity + scale + translateY -4px，150ms ease-emphasized
 *  - 浮层 exit：反向 100ms ease-in
 *  - item hover/focused：背景 tertiary（danger 时 danger 背景）
 *  - W4：浮层打开时监听 scroll，scroll 时关闭浮层
 * a11y：role=menu/menuitem/separator + aria-haspopup/expanded/controls
 */
import { computed, nextTick, onMounted, onUnmounted, ref, useId, watch } from 'vue'
import type { DropdownEmits, DropdownItem, DropdownProps } from './types'

const props = withDefaults(defineProps<DropdownProps>(), {
  placement: 'bottom-end',
  width: 200,
  trigger: 'trigger',
  triggerAction: 'click',
  hoverDelay: 100,
})

const emit = defineEmits<DropdownEmits>()

const internalId = useId()
const menuId = computed<string>(() => `ip-dropdown-${internalId}-menu`)

const open = ref<boolean>(props.modelValue)
const focusedIndex = ref<number>(-1)
const triggerRef = ref<HTMLElement | null>(null)
const popoverRef = ref<HTMLElement | null>(null)
const triggerRect = ref<DOMRect | null>(null)
let hoverTimer: ReturnType<typeof setTimeout> | null = null
let leaveTimer: ReturnType<typeof setTimeout> | null = null

/* 监听 modelValue 同步 */
watch(
  () => props.modelValue,
  (val) => {
    open.value = val
    if (val) {
      focusedIndex.value = findFirstNavigableIndex(0)
      measureTrigger()
      emit('open')
    } else {
      emit('close')
    }
  },
)

/* ----- 派生 ----- */
const navigableItems = computed(() =>
  props.items
    .map((item, idx) => ({ item, idx }))
    .filter(({ item }) => item.type === 'item' && !item.disabled),
)

/* ----- 索引工具 ----- */
function findFirstNavigableIndex(start: number): number {
  for (let i = start; i < props.items.length; i++) {
    const it = props.items[i] as Extract<DropdownItem, { type?: 'item' }>
    if (it.type === 'item' && !it.disabled) return i
  }
  return -1
}
function findLastNavigableIndex(): number {
  for (let i = props.items.length - 1; i >= 0; i--) {
    const it = props.items[i] as Extract<DropdownItem, { type?: 'item' }>
    if (it.type === 'item' && !it.disabled) return i
  }
  return -1
}

/* ----- 打开 / 关闭 ----- */
function openMenu(): void {
  if (open.value) return
  open.value = true
  emit('update:modelValue', true)
}
function closeMenu(): void {
  if (!open.value) return
  open.value = false
  focusedIndex.value = -1
  emit('update:modelValue', false)
  triggerRef.value?.focus?.()
}

/* ----- 选中 ----- */
async function selectItem(item: DropdownItem, index: number): Promise<void> {
  if (item.type !== 'item' || item.disabled) return
  emit('select', item, index)
  if (item.onClick) await item.onClick()
  if (!item.keepOpen) closeMenu()
}

/* ----- 键盘 ----- */
function onKeydown(ev: KeyboardEvent): void {
  if (!open.value) {
    if (ev.key === 'Enter' || ev.key === ' ' || ev.key === 'ArrowDown') {
      ev.preventDefault()
      openMenu()
    }
    return
  }
  switch (ev.key) {
    case 'Escape':
      ev.preventDefault()
      closeMenu()
      break
    case 'ArrowDown':
      ev.preventDefault()
      moveFocus(1)
      break
    case 'ArrowUp':
      ev.preventDefault()
      moveFocus(-1)
      break
    case 'Home':
      ev.preventDefault()
      focusedIndex.value = findFirstNavigableIndex(0)
      scrollFocusedIntoView()
      break
    case 'End':
      ev.preventDefault()
      focusedIndex.value = findLastNavigableIndex()
      scrollFocusedIntoView()
      break
    case 'Enter':
    case ' ':
      ev.preventDefault()
      if (focusedIndex.value >= 0)
        selectItem(props.items[focusedIndex.value] as DropdownItem, focusedIndex.value)
      break
    case 'Tab':
      closeMenu()
      break
  }
}
function moveFocus(delta: number): void {
  const list = navigableItems.value
  if (list.length === 0) return
  const currentPos = list.findIndex(({ idx }) => idx === focusedIndex.value)
  let nextPos = currentPos + delta
  if (nextPos < 0) nextPos = list.length - 1
  if (nextPos >= list.length) nextPos = 0
  focusedIndex.value = list[nextPos].idx
  scrollFocusedIntoView()
}
function scrollFocusedIntoView(): void {
  nextTick(() => {
    const popover = popoverRef.value
    if (!popover) return
    const active = popover.querySelector<HTMLElement>('.ip-dropdown__item--focused')
    active?.scrollIntoView({ block: 'nearest' })
  })
}

/* ----- 触发器交互 ----- */
function onTriggerClick(): void {
  if (props.triggerAction === 'click') {
    if (open.value) closeMenu()
    else openMenu()
  }
}
function onTriggerMouseEnter(): void {
  if (props.triggerAction !== 'hover') return
  if (leaveTimer) {
    clearTimeout(leaveTimer)
    leaveTimer = null
  }
  if (hoverTimer) clearTimeout(hoverTimer)
  hoverTimer = setTimeout(() => openMenu(), props.hoverDelay)
}
function onTriggerMouseLeave(): void {
  if (props.triggerAction !== 'hover') return
  if (hoverTimer) {
    clearTimeout(hoverTimer)
    hoverTimer = null
  }
  // 关闭：延迟 150ms，给用户移到菜单的时间
  leaveTimer = setTimeout(() => {
    if (popoverRef.value?.matches(':hover')) return
    closeMenu()
  }, 150)
}
function onPopoverMouseEnter(): void {
  if (leaveTimer) {
    clearTimeout(leaveTimer)
    leaveTimer = null
  }
}
function onPopoverMouseLeave(): void {
  if (props.triggerAction !== 'hover') return
  leaveTimer = setTimeout(() => closeMenu(), 150)
}

/* ----- 点击外部关闭 ----- */
function onDocumentMousedown(ev: MouseEvent): void {
  if (!open.value) return
  const target = ev.target as Node
  if (triggerRef.value?.contains(target)) return
  if (popoverRef.value?.contains(target)) return
  closeMenu()
}

/* W4：浮层打开时监听 scroll，scroll 时关闭浮层 */
function onWindowScroll(): void {
  if (open.value) closeMenu()
}

onMounted(() => {
  document.addEventListener('mousedown', onDocumentMousedown)
  window.addEventListener('scroll', onWindowScroll, true)
  /* P1-7 fix：初始 modelValue=true 时 watch 不会触发，需手动初始化 */
  if (props.modelValue) {
    measureTrigger()
    focusedIndex.value = findFirstNavigableIndex(0)
  }
})
onUnmounted(() => {
  document.removeEventListener('mousedown', onDocumentMousedown)
  window.removeEventListener('scroll', onWindowScroll, true)
  if (hoverTimer) clearTimeout(hoverTimer)
  if (leaveTimer) clearTimeout(leaveTimer)
})

/* ----- 浮层定位 ----- */
function measureTrigger(): void {
  if (triggerRef.value) triggerRect.value = triggerRef.value.getBoundingClientRect()
}

const popoverStyle = computed<Record<string, string>>(() => {
  const rect = triggerRect.value
  if (!rect) return {}
  const isTop = props.placement.startsWith('top')
  const isEnd = props.placement.endsWith('end')
  const GAP = 4
  const top = isTop ? `${rect.top - GAP}px` : `${rect.bottom + GAP}px`
  const styles: Record<string, string> = {
    position: 'fixed',
    top,
    minWidth: '180px',
    maxWidth: '280px',
    zIndex: 'var(--ip-z-dropdown)',
  }
  if (typeof props.width === 'number') styles.width = `${props.width}px`
  else styles.width = props.width
  if (isEnd) {
    styles.right = `${Math.max(0, window.innerWidth - rect.right)}px`
  } else {
    styles.left = `${Math.max(0, rect.left)}px`
  }
  if (isTop) styles.transform = 'translateY(-100%)'
  return styles
})
</script>

<template>
  <div class="ip-dropdown" @keydown="onKeydown">
    <div
      ref="triggerRef"
      class="ip-dropdown__trigger"
      :aria-haspopup="'menu'"
      :aria-expanded="open"
      :aria-controls="open ? menuId : undefined"
      @click="onTriggerClick"
      @mouseenter="onTriggerMouseEnter"
      @mouseleave="onTriggerMouseLeave"
    >
      <slot name="trigger" />
    </div>

    <Teleport to="body">
      <Transition name="ip-dropdown__popover">
        <div
          v-if="open"
          :id="menuId"
          ref="popoverRef"
          class="ip-dropdown__popover"
          :style="popoverStyle"
          role="menu"
          aria-label="操作"
          @mouseenter="onPopoverMouseEnter"
          @mouseleave="onPopoverMouseLeave"
        >
          <template v-for="(item, idx) in items" :key="item.key ?? idx">
            <div
              v-if="item.type === 'divider'"
              class="ip-dropdown__divider"
              role="separator"
            />
            <div
              v-else-if="item.type === 'label'"
              class="ip-dropdown__label"
              role="presentation"
            >{{ item.text }}</div>
            <div
              v-else
              :class="[
                'ip-dropdown__item',
                {
                  'ip-dropdown__item--disabled': item.disabled,
                  'ip-dropdown__item--danger': item.danger,
                  'ip-dropdown__item--focused': idx === focusedIndex,
                },
              ]"
              role="menuitem"
              :aria-disabled="item.disabled || undefined"
              @click="selectItem(item, idx)"
              @mouseenter="focusedIndex = idx"
            >
              <component
                :is="item.icon"
                v-if="item.icon"
                class="ip-dropdown__item-icon"
                :size="14"
                aria-hidden="true"
              />
              <span class="ip-dropdown__item-label">{{ item.label }}</span>
              <span v-if="item.shortcut" class="ip-dropdown__item-shortcut">{{ item.shortcut }}</span>
            </div>
          </template>
        </div>
      </Transition>
    </Teleport>

    <slot />
  </div>
</template>

<style scoped>
/* ============================================================
 * Dropdown — 根节点
 * ============================================================ */
.ip-dropdown {
  display: inline-flex;
  position: relative;
  font-family: inherit;
}

/* ============================================================
 * Trigger（容器：仅定位 + a11y 属性；视觉由消费者 slot 控制）
 * ============================================================ */
.ip-dropdown__trigger {
  display: inline-flex;
  align-items: center;
  outline: none;
}

/* ============================================================
 * Popover（§5.4.2）
 * ============================================================ */
.ip-dropdown__popover {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md);
  padding: var(--ip-spacing-1);
  max-height: var(--ip-dropdown-popover-max-h);
  overflow-y: auto;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  box-sizing: border-box;
  scrollbar-width: thin;
  scrollbar-color: var(--ip-color-bg-tertiary) transparent;
}
.ip-dropdown__popover::-webkit-scrollbar { width: 6px; }
.ip-dropdown__popover::-webkit-scrollbar-thumb {
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-full);
}

/* enter / leave（§5.5） */
.ip-dropdown__popover-enter-active {
  animation: ip-popover-in var(--ip-duration-base) var(--ip-ease-emphasized);
}
.ip-dropdown__popover-leave-active {
  animation: ip-popover-out var(--ip-duration-fast) var(--ip-ease-in);
}

/* ============================================================
 * Item（§5.4.3）
 * ============================================================ */
.ip-dropdown__item {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  min-height: var(--ip-dropdown-item-h);
  padding: var(--ip-spacing-1_5) var(--ip-spacing-3);
  border-radius: var(--ip-radius-sm);
  color: var(--ip-color-text-body);
  cursor: pointer;
  user-select: none;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-dropdown__item:hover,
.ip-dropdown__item--focused {
  background: var(--ip-color-bg-tertiary);
}
.ip-dropdown__item--focused {
  outline: 2px solid var(--ip-primary-500);
  outline-offset: -2px;
}

.ip-dropdown__item--disabled {
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
  pointer-events: none;
}

.ip-dropdown__item--danger {
  color: var(--ip-danger-text);
}
.ip-dropdown__item--danger:hover,
.ip-dropdown__item--danger.ip-dropdown__item--focused {
  background: var(--ip-danger-bg);
}

.ip-dropdown__item-icon {
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
}
.ip-dropdown__item--danger .ip-dropdown__item-icon {
  color: var(--ip-danger-text);
}

.ip-dropdown__item-label {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ip-dropdown__item-shortcut {
  flex-shrink: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  font-family: var(--ip-font-mono);
  margin-left: var(--ip-spacing-2);
}

/* ============================================================
 * Divider（§5.4.4）
 * ============================================================ */
.ip-dropdown__divider {
  height: 1px;
  background: var(--ip-color-border-default);
  margin: var(--ip-spacing-1) 0;
  border: none;
}

/* ============================================================
 * Label（§5.4.5）
 * ============================================================ */
.ip-dropdown__label {
  padding: var(--ip-spacing-1_5) var(--ip-spacing-3) var(--ip-spacing-0_5);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-tertiary);
  user-select: none;
  pointer-events: none;
}
</style>