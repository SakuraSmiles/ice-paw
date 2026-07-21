<script setup lang="ts" generic="V extends string | number">
/**
 * Select — IcePaw 下拉选择（受控单选）
 *
 * 规范：icepaw-p0-component-specs.md §三
 * 微交互：
 *  - 浮层 enter：opacity + scale + translateY -4px，150ms ease-emphasized
 *  - 浮层 exit：反向 100ms ease-in
 *  - ChevronDown open 时旋转 180°
 *  - clearable ✕ hover/focus-within 时 fade-in 150ms
 *  - W4：浮层打开时监听 scroll，scroll 时关闭浮层
 * a11y：role=combobox/listbox/option + aria-expanded/controls/activedescendant
 *
 * 泛型：W3 用 `<script setup lang="ts" generic="V extends string | number">`
 */
import { computed, nextTick, onMounted, onUnmounted, ref, useId } from 'vue'
import { Check, ChevronDown, X } from 'lucide-vue-next'
import type { SelectEmits, SelectOption, SelectProps } from './types'

const props = withDefaults(defineProps<SelectProps<V>>(), {
  size: 'md',
  disabled: false,
  error: false,
  clearable: false,
  placement: 'bottom-start',
  popoverWidth: 'match-trigger',
})

const emit = defineEmits<SelectEmits<V>>()

const internalId = useId()
const selectId = computed<string>(() => props.id ?? `ip-select-${internalId}`)
const listboxId = computed<string>(() => `${selectId.value}-listbox`)

const open = ref<boolean>(false)
const focusedIndex = ref<number>(-1)
const hovered = ref<boolean>(false)
const triggerRef = ref<HTMLElement | null>(null)
const popoverRef = ref<HTMLElement | null>(null)
const triggerWidth = ref<number>(0)

/* ----- 派生 ----- */
const selectedOption = computed<SelectOption<V> | undefined>(() =>
  props.options.find((o) => o.value === props.modelValue),
)
const canClear = computed<boolean>(
  () =>
    props.clearable &&
    props.modelValue !== null &&
    props.modelValue !== undefined &&
    !props.disabled,
)
/* P1-1 fix：✕ 在浮层打开 / hover / 键盘聚焦 时都显示（§3.4.3）
   原条件 `open.value && (hovered.value || open.value)` 等价于 open.value，
   导致 ✕ 仅在浮层打开时出现，hover/focus-within 时无 ✕。 */
const showClear = computed<boolean>(() => canClear.value && (hovered.value || open.value))

/* ----- 打开 / 关闭 ----- */
function openPopover(): void {
  if (props.disabled) return
  if (open.value) return
  open.value = true
  // 默认 active index = 当前 selected
  const idx = props.options.findIndex((o) => o.value === props.modelValue)
  focusedIndex.value = idx >= 0 ? idx : 0
  measureTrigger()
  emit('open')
}
function closePopover(): void {
  if (!open.value) return
  open.value = false
  focusedIndex.value = -1
  emit('close')
}
function measureTrigger(): void {
  if (triggerRef.value) triggerWidth.value = triggerRef.value.offsetWidth
}
function togglePopover(): void {
  if (open.value) {
    closePopover()
  } else {
    openPopover()
  }
}

/* ----- 选中 ----- */
function selectOption(opt: SelectOption<V>): void {
  if (opt.disabled) return
  emit('update:modelValue', opt.value)
  emit('change', opt.value)
  closePopover()
  triggerRef.value?.focus()
}
function clearValue(ev: MouseEvent): void {
  ev.stopPropagation()
  if (props.disabled) return
  emit('update:modelValue', null)
  emit('clear')
  // P0-1 fix：stopPropagation 阻止冒泡到 trigger 的 @click，浮层未打开时主动打开
  if (!open.value) openPopover()
}

/* ----- 键盘 ----- */
function onKeydown(ev: KeyboardEvent): void {
  if (props.disabled) return
  switch (ev.key) {
    case 'Enter':
      ev.preventDefault()
      if (!open.value) openPopover()
      else if (focusedIndex.value >= 0) selectOption(props.options[focusedIndex.value] as SelectOption<V>)
      break
    case ' ':
      // §3.6.2：禁用 Space 打开（仅 Enter 打开），open 状态下 Space 也直接选中
      ev.preventDefault()
      if (open.value && focusedIndex.value >= 0) {
        selectOption(props.options[focusedIndex.value] as SelectOption<V>)
      }
      break
    case 'Escape':
      if (open.value) {
        ev.preventDefault()
        closePopover()
      }
      break
    case 'ArrowDown':
      ev.preventDefault()
      if (!open.value) openPopover()
      else moveFocus(1)
      break
    case 'ArrowUp':
      ev.preventDefault()
      if (!open.value) openPopover()
      else moveFocus(-1)
      break
    case 'Home':
      if (open.value) {
        ev.preventDefault()
        focusedIndex.value = 0
        scrollActiveIntoView()
      }
      break
    case 'End':
      if (open.value) {
        ev.preventDefault()
        focusedIndex.value = props.options.length - 1
        scrollActiveIntoView()
      }
      break
    case 'Tab':
      closePopover()
      break
  }
}
function moveFocus(delta: number): void {
  if (props.options.length === 0) return
  let next = focusedIndex.value + delta
  if (next < 0) next = props.options.length - 1
  if (next >= props.options.length) next = 0
  // 跳过 disabled
  let attempts = 0
  while ((props.options[next] as SelectOption<V>).disabled && attempts < props.options.length) {
    next = (next + delta + props.options.length) % props.options.length
    attempts++
  }
  focusedIndex.value = next
  scrollActiveIntoView()
}
function scrollActiveIntoView(): void {
  nextTick(() => {
    const popover = popoverRef.value
    if (!popover) return
    const active = popover.querySelector<HTMLElement>('.ip-select__option--focused')
    active?.scrollIntoView({ block: 'nearest' })
  })
}

/* ----- 点击外部关闭 ----- */
function onDocumentClick(ev: MouseEvent): void {
  if (!open.value) return
  const target = ev.target as Node
  if (triggerRef.value?.contains(target)) return
  if (popoverRef.value?.contains(target)) return
  closePopover()
}

/* W4：浮层打开时监听 scroll，scroll 时关闭浮层
   P0-1 fix：忽略浮层内部的滚动（overflow-y: auto），避免滚动选项列表时误关 */
function onWindowScroll(ev: Event): void {
  if (!open.value) return
  if (popoverRef.value?.contains(ev.target as Node)) return
  closePopover()
}

onMounted(() => {
  document.addEventListener('click', onDocumentClick)
  window.addEventListener('scroll', onWindowScroll, true)
})
onUnmounted(() => {
  document.removeEventListener('click', onDocumentClick)
  window.removeEventListener('scroll', onWindowScroll, true)
})

/* ----- 浮层定位（简化：fixed + 计算）----- */
const popoverStyle = computed<Record<string, string>>(() => {
  if (!triggerRef.value) return {}
  const rect = triggerRef.value.getBoundingClientRect()
  const isTop = props.placement.startsWith('top')
  const isEnd = props.placement.endsWith('end')
  const GAP = 4
  const top = isTop ? `${rect.top - GAP}px` : `${rect.bottom + GAP}px`
  let width: string
  if (props.popoverWidth === 'match-trigger') width = `${triggerWidth.value}px`
  else if (typeof props.popoverWidth === 'number') width = `${props.popoverWidth}px`
  else if (typeof props.popoverWidth === 'string') width = props.popoverWidth
  else width = 'auto'
  const styles: Record<string, string> = {
    position: 'fixed',
    top,
    width,
    zIndex: 'var(--ip-z-dropdown)',
  }
  if (isEnd) {
    styles.right = `${Math.max(0, window.innerWidth - rect.right)}px`
  } else {
    styles.left = `${Math.max(0, rect.left)}px`
  }
  if (isTop) {
    styles.transform = 'translateY(-100%)'
  }
  return styles
})
</script>

<template>
  <div
    :class="[
      'ip-select',
      `ip-select--${size}`,
      {
        'ip-select--open': open,
        'ip-select--disabled': disabled,
        'ip-select--error': error,
        'ip-select--focused': open,
        'ip-select--hovered': hovered,
      },
    ]"
  >
    <div
      ref="triggerRef"
      :class="['ip-select__trigger', `ip-select__trigger--${size}`]"
      role="combobox"
      :aria-expanded="open"
      aria-haspopup="listbox"
      :aria-controls="listboxId"
      :aria-activedescendant="open && focusedIndex >= 0 ? `${selectId}-opt-${focusedIndex}` : undefined"
      :aria-disabled="disabled || undefined"
      :tabindex="disabled ? -1 : 0"
      @click="togglePopover"
      @keydown="onKeydown"
      @mouseenter="hovered = true"
      @mouseleave="hovered = false"
    >
      <component
        :is="prefixIcon"
        v-if="prefixIcon"
        class="ip-select__prefix"
        :size="16"
        aria-hidden="true"
      />

      <span v-if="selectedOption" class="ip-select__value">
        <component
          :is="selectedOption.icon"
          v-if="selectedOption.icon"
          :size="14"
          aria-hidden="true"
        />
        {{ selectedOption.label }}
      </span>
      <span v-else class="ip-select__placeholder">{{ placeholder ?? '请选择' }}</span>

      <button
        v-if="showClear"
        type="button"
        class="ip-select__clear"
        aria-label="清空"
        tabindex="-1"
        @click="clearValue"
      >
        <X :size="14" aria-hidden="true" />
      </button>

      <ChevronDown :size="14" class="ip-select__chevron" aria-hidden="true" />
    </div>

    <Teleport to="body">
      <Transition name="ip-select__popover">
        <div
          v-if="open"
          :id="listboxId"
          ref="popoverRef"
          class="ip-select__popover"
          :style="popoverStyle"
          role="listbox"
          :aria-label="ariaLabel ?? '选项'"
        >
          <div
            v-for="(opt, idx) in options"
            :id="`${selectId}-opt-${idx}`"
            :key="String(opt.value)"
            :class="[
              'ip-select__option',
              {
                'ip-select__option--selected': opt.value === modelValue,
                'ip-select__option--disabled': opt.disabled,
                'ip-select__option--focused': idx === focusedIndex,
              },
            ]"
            role="option"
            :aria-selected="opt.value === modelValue"
            :aria-disabled="opt.disabled || undefined"
            @click="selectOption(opt)"
            @mouseenter="focusedIndex = idx"
          >
            <component
              :is="opt.icon"
              v-if="opt.icon"
              class="ip-select__option-icon"
              :size="14"
              aria-hidden="true"
            />
            <span class="ip-select__option-label">{{ opt.label }}</span>
            <span
              v-if="opt.description"
              class="ip-select__option-description"
              :title="opt.description"
            >{{ opt.description }}</span>
            <Check
              v-if="opt.value === modelValue"
              class="ip-select__option-check"
              :size="14"
              aria-hidden="true"
            />
          </div>
        </div>
      </Transition>
    </Teleport>

    <span v-if="error && errorMessage" class="ip-select__error-text" role="alert">
      {{ errorMessage }}
    </span>
  </div>
</template>

<style scoped>
/* ============================================================
 * Select — 根节点
 * ============================================================ */
.ip-select {
  display: inline-flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
  width: 100%;
  position: relative;
  font-family: inherit;
}
.ip-select--block { width: 100%; }

/* ============================================================
 * Trigger（与 Input 视觉对齐 §3.4.1 / §3.4.2）
 * ============================================================ */
.ip-select__trigger {
  display: inline-flex;
  align-items: center;
  width: 100%;
  min-height: var(--ip-input-h-md);
  padding: var(--ip-input-py-md) var(--ip-input-px-md);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-input-radius);
  color: var(--ip-color-text-body);
  font-family: inherit;
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-line-height-loose);
  gap: var(--ip-spacing-2);
  cursor: pointer;
  user-select: none;
  outline: none;
  position: relative;

  transition:
    border-color var(--ip-duration-fast) var(--ip-ease-out),
    box-shadow var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}

/* 尺寸档（与 Input 一致） */
.ip-select__trigger--sm {
  min-height: var(--ip-input-h-sm);
  padding: var(--ip-input-py-sm) var(--ip-input-px-sm);
  font-size: var(--ip-text-body-sm-size);
}
.ip-select__trigger--md {
  min-height: var(--ip-input-h-md);
  padding: var(--ip-input-py-md) var(--ip-input-px-md);
  font-size: var(--ip-text-body-size);
}
.ip-select__trigger--lg {
  min-height: var(--ip-input-h-lg);
  padding: var(--ip-input-py-lg) var(--ip-input-px-lg);
  font-size: var(--ip-text-body-lg-size);
}

/* hover（§3.4.2） */
.ip-select--hovered:not(.ip-select--open):not(.ip-select--disabled) .ip-select__trigger {
  border-color: var(--ip-color-border-strong);
}

/* focus / open（§3.4.2） */
.ip-select--focused .ip-select__trigger {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
  color: var(--ip-color-text-primary);
}

/* error */
.ip-select--error .ip-select__trigger {
  border-color: var(--ip-color-border-error);
  /* P1-2 fix：error 状态 trigger 加 shake 动画（与 Input 对齐 §2.8） */
  animation: ip-input-shake var(--ip-duration-shake) var(--ip-ease-in-out);
}
.ip-select--error.ip-select--focused .ip-select__trigger {
  border-color: var(--ip-color-border-error);
  box-shadow: var(--ip-shadow-focus-danger);
}

/* disabled */
.ip-select--disabled .ip-select__trigger {
  /* P1-5 fix：使用语义 token 而非原始色板 */
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-default);
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
}

/* focus-visible 键盘环（trigger 自身的 focus 由原生 div:focus-visible 处理） */
.ip-select__trigger:focus-visible {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * Trigger 内元素
 * ============================================================ */
.ip-select__prefix {
  display: inline-flex;
  align-items: center;
  color: var(--ip-color-icon-muted);
  flex-shrink: 0;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-select--focused .ip-select__prefix {
  /* P1-5 fix：使用语义 token 而非原始色板 */
  color: var(--ip-color-text-secondary);
}

.ip-select__value {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  flex: 1;
  min-width: 0;
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ip-select__placeholder {
  flex: 1;
  min-width: 0;
  color: var(--ip-color-text-placeholder);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* clear ✕（§3.4.3，与 Input clear 同款） */
.ip-select__clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  margin-right: -4px;
  flex-shrink: 0;
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-full);
  color: var(--ip-color-icon-muted);
  cursor: pointer;
  transition:
    color var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-select__clear:hover {
  /* P1-5 fix：使用语义 token 而非原始色板 */
  color: var(--ip-color-text-body);
  background: var(--ip-color-bg-tertiary);
}

.ip-select__chevron {
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
  transition:
    color var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-base) var(--ip-ease-out);
}
.ip-select--open .ip-select__chevron {
  transform: rotate(180deg);
  color: var(--ip-color-icon-default);
}

/* ============================================================
 * Popover（§3.4.4 / Teleport 到 body）
 * ============================================================ */
.ip-select__popover {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md);
  padding: var(--ip-spacing-1);
  min-width: var(--ip-select-popover-min-w);
  max-width: var(--ip-select-popover-max-w);
  max-height: var(--ip-select-popover-max-h);
  overflow-y: auto;
  font-family: inherit;
  font-size: var(--ip-text-body-size);
  box-sizing: border-box;
  /* 6px 细滚动条 */
  scrollbar-width: thin;
  scrollbar-color: var(--ip-color-bg-tertiary) transparent;
}
.ip-select__popover::-webkit-scrollbar { width: 6px; }
.ip-select__popover::-webkit-scrollbar-thumb {
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-full);
}

/* enter / leave 动画（§3.5） */
.ip-select__popover-enter-active {
  animation: ip-popover-in var(--ip-duration-base) var(--ip-ease-emphasized);
}
.ip-select__popover-leave-active {
  animation: ip-popover-out var(--ip-duration-fast) var(--ip-ease-in);
}

/* ============================================================
 * Option（§3.4.5）
 * ============================================================ */
.ip-select__option {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  min-height: var(--ip-select-option-h);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border-radius: var(--ip-radius-sm);
  color: var(--ip-color-text-primary);
  cursor: pointer;
  user-select: none;
  position: relative;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-select__option--selected,
.ip-select__option--focused {
  background: var(--ip-color-bg-tertiary);
}
.ip-select__option--focused {
  outline: 2px solid var(--ip-primary-500);
  outline-offset: -2px;
}

.ip-select__option--disabled {
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
  pointer-events: none;
}

.ip-select__option-icon {
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
}

.ip-select__option-label {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ip-select__option-description {
  flex-shrink: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  max-width: 280px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ip-select__option-check {
  flex-shrink: 0;
  margin-left: auto;
  color: var(--ip-color-icon-default);
}

/* ============================================================
 * Error text
 * ============================================================ */
.ip-select__error-text {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-loose3);
  color: var(--ip-danger-text);
  animation: ip-error-msg-in var(--ip-duration-message) var(--ip-ease-out) both;
}
</style>