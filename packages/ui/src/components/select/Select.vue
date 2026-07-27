<script setup lang="ts" generic="V extends string | number">
/**
 * Select — IcePaw 下拉选择（受控单选 / 多选 / 搜索）
 *
 * 规范：icepaw-p0-component-specs.md §三
 *
 * 功能：
 *  - REQ-UI-003：单选基础（outline / filled 变体、clearable、prefixIcon）
 *  - REQ-UI-003B：loading 态（浮层 spinner + 禁用交互）
 *  - REQ-UI-003C：空列表显示 "暂无选项"
 *  - REQ-UI-003D：多选模式（multiple）— trigger 内 tag 列表 + tag × 取消 + 浮层 checkbox + 不自动关闭
 *  - REQ-UI-003E：搜索过滤（filterable）— 浮层顶部搜索框 + 实时过滤 + "无匹配选项"
 *
 * 微交互：
 *  - 浮层 enter：opacity + scale + translateY -4px，150ms ease-emphasized
 *  - 浮层 exit：反向 100ms ease-in
 *  - ChevronDown open 时旋转 180°
 *  - clearable ✕ hover/focus-within 时 fade-in 150ms
 *  - P3-fix：浮层打开时监听 scroll/resize（document capture），实时重算位置跟随 trigger，不关闭
 *
 * a11y：role=combobox/listbox/option + aria-expanded/controls/activedescendant
 * 多选 a11y：role=combobox + aria-multiselectable=true
 *
 * 泛型：W3 用 `<script setup lang="ts" generic="V extends string | number">`
 */
import { computed, nextTick, onBeforeUnmount, onMounted, onUnmounted, ref, useId } from 'vue'
import { Check, ChevronDown, Search, Square, X } from 'lucide-vue-next'
import type { SelectEmits, SelectOption, SelectProps } from './types'

const props = withDefaults(defineProps<SelectProps<V>>(), {
  size: 'md',
  variant: 'outline',
  disabled: false,
  error: false,
  clearable: false,
  loading: false,
  placement: 'bottom-start',
  popoverWidth: 'match-trigger',
  multiple: false,
  filterable: false,
  filter: undefined,
})

/* REQ-UI-003B：loading 时禁止打开/选中 */
const isInteractive = computed<boolean>(() => !props.disabled && !props.loading)

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

/* REQ-UI-003E：搜索值 */
const searchValue = ref<string>('')
const searchInputRef = ref<HTMLInputElement | null>(null)

/* ----- 派生 ----- */

/**
 * 当前选中值（统一处理单选 / 多选）
 *  - 单选：返回 `SelectOption<V> | undefined`
 *  - 多选：返回 `SelectOption<V>[]`
 */
const selectedOptions = computed<SelectOption<V>[]>(() => {
  if (props.multiple) {
    if (!Array.isArray(props.modelValue)) return []
    const set = new Set(props.modelValue as V[])
    return props.options.filter((o) => set.has(o.value))
  }
  const single = props.options.find((o) => o.value === props.modelValue)
  return single ? [single] : []
})

const selectedOption = computed<SelectOption<V> | undefined>(() =>
  props.multiple ? undefined : selectedOptions.value[0],
)

/* REQ-UI-003E：默认过滤函数 */
const defaultFilter = (q: string, opt: SelectOption<V>): boolean =>
  opt.label.toLowerCase().includes(q.toLowerCase())

/**
 * REQ-UI-003E：过滤后的选项列表
 *  - filterable=false → 原始 options
 *  - filterable=true 且 searchValue 为空 → 原始 options
 *  - filterable=true 且 searchValue 非空 → 按 filter(q, opt) 过滤
 */
const filteredOptions = computed<SelectOption<V>[]>(() => {
  if (!props.filterable) return props.options
  const q = searchValue.value.trim()
  if (q.length === 0) return props.options
  const fn = props.filter ?? defaultFilter
  return props.options.filter((o) => fn(q, o))
})

/**
 * 当前选项列表（用于 v-for 和焦点遍历）
 * 单选 / 多选 / 过滤都用同一个 filteredOptions
 */
const visibleOptions = computed<SelectOption<V>[]>(() => filteredOptions.value)

/**
 * REQ-UI-003C：空列表判断（区分"无任何选项"和"搜索无匹配"）
 */
const isEmptyOptions = computed<boolean>(() => props.options.length === 0)
const isNoMatch = computed<boolean>(
  () => props.filterable && props.options.length > 0 && filteredOptions.value.length === 0,
)

const canClear = computed<boolean>(() => {
  if (!props.clearable || props.disabled) return false
  if (props.multiple) {
    return Array.isArray(props.modelValue) && (props.modelValue as V[]).length > 0
  }
  return props.modelValue !== null && props.modelValue !== undefined
})

/* P1-1 fix：✕ 在浮层打开 / hover / 键盘聚焦 时都显示（§3.4.3）
   原条件 `open.value && (hovered.value || open.value)` 等价于 open.value，
   导致 ✕ 仅在浮层打开时出现，hover/focus-within 时无 ✕。 */
const showClear = computed<boolean>(() => canClear.value && (hovered.value || open.value))

/* ----- 打开 / 关闭 ----- */
function openPopover(): void {
  if (!isInteractive.value) return
  if (open.value) return
  open.value = true
  // 默认 active index = 当前 selected
  const list = visibleOptions.value
  if (props.multiple) {
    // 多选：默认 active index = 第一个选项
    focusedIndex.value = list.length > 0 ? 0 : -1
  } else {
    const idx = list.findIndex((o) => o.value === props.modelValue)
    focusedIndex.value = idx >= 0 ? idx : list.length > 0 ? 0 : -1
  }
  // REQ-UI-003E：浮层打开时清空搜索值（避免上次残留）
  if (props.filterable) {
    searchValue.value = ''
  }
  measureTrigger()
  // P2-fix：用 nextTick 等 DOM 渲染后命令式重算 popover 位置
  nextTick(() => {
    updatePopoverPosition()
    // REQ-UI-003E：搜索框自动聚焦
    if (props.filterable) {
      searchInputRef.value?.focus()
    }
  })
  // 同步监听 trigger 大小变化
  if (triggerRef.value && !resizeObserver) {
    resizeObserver = new ResizeObserver(() => {
      if (open.value) updatePopoverPosition()
    })
    resizeObserver.observe(triggerRef.value)
  }
  emit('open')
}
function closePopover(): void {
  if (!open.value) return
  open.value = false
  focusedIndex.value = -1
  // REQ-UI-003E：关闭时清空搜索值
  if (props.filterable) {
    searchValue.value = ''
  }
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

/**
 * 判断 option 是否被选中（单选 / 多选通用）
 */
function isSelected(opt: SelectOption<V>): boolean {
  if (props.multiple) {
    return Array.isArray(props.modelValue) && (props.modelValue as V[]).includes(opt.value)
  }
  return opt.value === props.modelValue
}

/* ----- 选中 ----- */
function selectOption(opt: SelectOption<V>): void {
  if (opt.disabled || props.loading) return
  if (props.multiple) {
    const current = Array.isArray(props.modelValue) ? ([...props.modelValue] as V[]) : []
    const idx = current.indexOf(opt.value)
    if (idx >= 0) {
      current.splice(idx, 1)
    } else {
      current.push(opt.value)
    }
    emit('update:modelValue', current)
    emit('change', current)
    // REQ-UI-003D：多选模式浮层不自动关闭
    // REQ-UI-003E：多选选中后清空搜索值（更接近常见 UX）
    if (props.filterable) {
      searchValue.value = ''
      nextTick(() => searchInputRef.value?.focus())
    }
    return
  }
  emit('update:modelValue', opt.value)
  emit('change', opt.value)
  closePopover()
  triggerRef.value?.focus()
}

/**
 * REQ-UI-003D：多选 tag × 按钮 — 取消单个选项
 */
function removeTag(value: V, ev: Event): void {
  ev.stopPropagation()
  if (props.disabled || props.loading) return
  if (!props.multiple || !Array.isArray(props.modelValue)) return
  const current = (props.modelValue as V[]).filter((v) => v !== value)
  emit('update:modelValue', current)
  emit('change', current)
  emit('remove-tag', value)
}

function clearValue(ev: MouseEvent): void {
  ev.stopPropagation()
  if (props.disabled) return
  if (props.multiple) {
    emit('update:modelValue', [])
    emit('clear')
  } else {
    emit('update:modelValue', null)
    emit('clear')
  }
  // P0-1 fix：stopPropagation 阻止冒泡到 trigger 的 @click，浮层未打开时主动打开
  if (!open.value) openPopover()
}

/* ----- 键盘 ----- */
function onKeydown(ev: KeyboardEvent): void {
  if (!isInteractive.value) return
  // REQ-UI-003E：搜索框聚焦时不接管键盘（避免 ArrowDown/Escape 抢逻辑）
  const isSearchFocused = ev.target === searchInputRef.value
  switch (ev.key) {
    case 'Enter':
      // REQ-UI-003E：搜索框中 Enter 不抢逻辑（防止搜索框 Enter 关闭浮层）
      if (isSearchFocused && props.filterable) {
        // Enter 在搜索框：若唯一匹配项则选中
        const list = visibleOptions.value
        if (list.length === 1 && focusedIndex.value === 0) {
          ev.preventDefault()
          selectOption(list[0] as SelectOption<V>)
        }
        return
      }
      ev.preventDefault()
      if (!open.value) openPopover()
      else if (focusedIndex.value >= 0) {
        const opt = visibleOptions.value[focusedIndex.value]
        if (opt) selectOption(opt)
      }
      break
    case ' ':
      // §3.6.2：禁用 Space 打开（仅 Enter 打开），open 状态下 Space 也直接选中
      if (isSearchFocused && props.filterable) return
      ev.preventDefault()
      if (open.value && focusedIndex.value >= 0) {
        const opt = visibleOptions.value[focusedIndex.value]
        if (opt) selectOption(opt)
      }
      break
    case 'Escape':
      if (open.value) {
        // REQ-UI-003E：搜索框有输入时，先清空搜索，再关闭浮层
        if (isSearchFocused && props.filterable && searchValue.value.length > 0) {
          searchValue.value = ''
          return
        }
        ev.preventDefault()
        closePopover()
      }
      break
    case 'ArrowDown':
      if (isSearchFocused && props.filterable) return
      ev.preventDefault()
      if (!open.value) openPopover()
      else moveFocus(1)
      break
    case 'ArrowUp':
      if (isSearchFocused && props.filterable) return
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
        focusedIndex.value = visibleOptions.value.length - 1
        scrollActiveIntoView()
      }
      break
    case 'Tab':
      closePopover()
      break
    case 'Backspace':
      // REQ-UI-003D：多选模式下，trigger 聚焦时按 Backspace 删除最后一个 tag
      if (
        props.multiple &&
        !props.filterable /* 搜索框聚焦时不触发 */ &&
        Array.isArray(props.modelValue) &&
        (props.modelValue as V[]).length > 0
      ) {
        ev.preventDefault()
        const last = (props.modelValue as V[])[(props.modelValue as V[]).length - 1] as V
        removeTag(last, ev)
      }
      break
  }
}
function moveFocus(delta: number): void {
  const list = visibleOptions.value
  if (list.length === 0) return
  let next = focusedIndex.value + delta
  if (next < 0) next = list.length - 1
  if (next >= list.length) next = 0
  // 跳过 disabled
  let attempts = 0
  while ((list[next] as SelectOption<V>).disabled && attempts < list.length) {
    next = (next + delta + list.length) % list.length
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

/* P3-fix：scroll/resize 时实时重算 popover 位置（跟随 trigger），不关闭
   - capture 阶段 + document 监听：捕获所有滚动容器（包括自定义滚动容器）
   - 忽略浮层内部滚动（overflow-y: auto 选项列表）
   - window resize 时同步重算（trigger 大小已由 ResizeObserver 处理，但 viewport 变化时也要重算） */
function onScrollOrResize(ev: Event): void {
  if (!open.value) return
  if (popoverRef.value?.contains(ev.target as Node)) return  // 浮层自身滚动忽略
  updatePopoverPosition()
}

onMounted(() => {
  document.addEventListener('click', onDocumentClick)
  // P3-fix：document capture 监听 scroll + window 监听 resize，捕获所有滚动源
  document.addEventListener('scroll', onScrollOrResize, { capture: true, passive: true })
  window.addEventListener('resize', onScrollOrResize)
})

onBeforeUnmount(() => {
  // 清理 ResizeObserver
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
})

onUnmounted(() => {
  document.removeEventListener('click', onDocumentClick)
  document.removeEventListener('scroll', onScrollOrResize, { capture: true })
  window.removeEventListener('resize', onScrollOrResize)
})

/* ----- 浮层定位（命令式重算 + ResizeObserver）-----
   P2-fix：watchEffect 不会响应 getBoundingClientRect 变化（不是响应式依赖），
   改为 nextTick 显式调用 + ResizeObserver 同步 trigger 大小 */
const popoverStyle = ref<Record<string, string>>({})
let resizeObserver: ResizeObserver | null = null

function updatePopoverPosition(): void {
  if (!triggerRef.value || !open.value) {
    popoverStyle.value = {}
    return
  }
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
  popoverStyle.value = styles
}
</script>

<template>
  <div
    :class="[
      'ip-select',
      `ip-select--${size}`,
      `ip-select--${variant}`,
      {
        'ip-select--open': open,
        'ip-select--disabled': disabled,
        'ip-select--error': error,
        'ip-select--focused': open,
        'ip-select--hovered': hovered,
        'ip-select--loading': loading,
        'ip-select--multiple': multiple,
      },
    ]"
  >
    <div
      ref="triggerRef"
      :class="['ip-select__trigger', `ip-select__trigger--${size}`]"
      role="combobox"
      :aria-expanded="open"
      :aria-haspopup="multiple ? 'listbox' : 'listbox'"
      :aria-controls="listboxId"
      :aria-activedescendant="open && focusedIndex >= 0 ? `${selectId}-opt-${focusedIndex}` : undefined"
      :aria-disabled="disabled || loading || undefined"
      :aria-busy="loading || undefined"
      :aria-multiselectable="multiple || undefined"
      :tabindex="disabled || loading ? -1 : 0"
      @click="togglePopover"
      @keydown="onKeydown"
      @mouseenter="hovered = true"
      @mouseleave="hovered = false"
    >
      <component
        :is="prefixIcon"
        v-if="prefixIcon && !multiple"
        class="ip-select__prefix"
        :size="16"
        aria-hidden="true"
      />

      <!-- REQ-UI-003D：多选 trigger 区域（tag 列表） -->
      <div v-if="multiple" class="ip-select__tags">
        <template v-if="selectedOptions.length > 0">
          <span
            v-for="opt in selectedOptions"
            :key="String(opt.value)"
            class="ip-select__tag"
            :class="{ 'ip-select__tag--disabled': disabled }"
          >
            <component
              :is="opt.icon"
              v-if="opt.icon"
              :size="12"
              class="ip-select__tag-icon"
              aria-hidden="true"
            />
            <span class="ip-select__tag-label">{{ opt.label }}</span>
            <button
              v-if="!disabled && !loading"
              type="button"
              class="ip-select__tag-close"
              :aria-label="`移除 ${opt.label}`"
              tabindex="-1"
              @click="removeTag(opt.value, $event)"
            >
              <X :size="12" aria-hidden="true" />
            </button>
          </span>
        </template>
        <!-- 隐藏的 placeholder：占位以维持 trigger 高度 -->
        <span v-else class="ip-select__placeholder">{{ placeholder ?? '请选择' }}</span>
      </div>

      <!-- 单选 trigger 区域（保持原行为） -->
      <span v-else-if="selectedOption" class="ip-select__value">
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
        :aria-label="multiple ? '清空全部' : '清空'"
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
          :class="{ 'ip-select__popover--filterable': filterable }"
          :style="popoverStyle"
          role="listbox"
          :aria-label="ariaLabel ?? '选项'"
          :aria-busy="loading || undefined"
          :aria-multiselectable="multiple || undefined"
        >
          <!-- REQ-UI-003E：搜索框 -->
          <div v-if="filterable && !loading" class="ip-select__search">
            <Search :size="14" class="ip-select__search-icon" aria-hidden="true" />
            <input
              ref="searchInputRef"
              v-model="searchValue"
              type="text"
              class="ip-select__search-input"
              :placeholder="searchPlaceholder ?? '搜索选项…'"
              autocomplete="off"
              spellcheck="false"
              @keydown.stop="onKeydown"
              @input="emit('search', searchValue)"
            />
            <button
              v-if="searchValue.length > 0"
              type="button"
              class="ip-select__search-clear"
              aria-label="清空搜索"
              tabindex="-1"
              @click="searchValue = ''"
            >
              <X :size="12" aria-hidden="true" />
            </button>
          </div>

          <!-- REQ-UI-003B：loading spinner in popover -->
          <div v-if="loading" class="ip-select__loading">
            <svg class="ip-select__spinner" width="20" height="20" viewBox="0 0 24 24" fill="none">
              <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" stroke-linecap="round" opacity="0.25" />
              <path d="M12 2a10 10 0 0 1 10 10" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
            </svg>
            <span class="ip-select__loading-text">加载中…</span>
          </div>
          <template v-else>
            <!-- REQ-UI-003C：空列表显示空状态文案 -->
            <div v-if="isEmptyOptions" class="ip-select__empty">
              <span class="ip-select__empty-text">{{ emptyText ?? '暂无选项' }}</span>
            </div>
            <!-- REQ-UI-003E：搜索无结果 -->
            <div v-else-if="isNoMatch" class="ip-select__empty">
              <span class="ip-select__empty-text">{{ noMatchText ?? '无匹配选项' }}</span>
            </div>
            <template v-else>
              <div
                v-for="(opt, idx) in visibleOptions"
                :id="`${selectId}-opt-${idx}`"
                :key="String(opt.value)"
                :class="[
                  'ip-select__option',
                  {
                    'ip-select__option--selected': isSelected(opt),
                    'ip-select__option--disabled': opt.disabled,
                    'ip-select__option--focused': idx === focusedIndex,
                    'ip-select__option--multiple': multiple,
                  },
                ]"
                role="option"
                :aria-selected="isSelected(opt)"
                :aria-disabled="opt.disabled || undefined"
                @click="selectOption(opt)"
                @mouseenter="focusedIndex = idx"
              >
                <!-- REQ-UI-003D：多选模式 checkbox 视觉 -->
                <span
                  v-if="multiple"
                  class="ip-select__option-checkbox"
                  :class="{ 'ip-select__option-checkbox--checked': isSelected(opt) }"
                  aria-hidden="true"
                >
                  <Check v-if="isSelected(opt)" :size="12" />
                  <Square v-else :size="14" />
                </span>
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
                <!-- 单选模式保留 ✓ 标记 -->
                <Check
                  v-if="!multiple && isSelected(opt)"
                  class="ip-select__option-check"
                  :size="14"
                  aria-hidden="true"
                />
              </div>
            </template>
          </template>
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

/* REQ-UI-003：filled 变体 */
.ip-select--filled .ip-select__trigger {
  border-color: transparent;
  background: var(--ip-color-bg-tertiary);
}
.ip-select--filled.ip-select--hovered:not(.ip-select--open):not(.ip-select--disabled) .ip-select__trigger {
  background: var(--ip-gray-200);
  border-color: transparent;
}
.ip-select--filled.ip-select--focused .ip-select__trigger {
  background: var(--ip-color-bg-tertiary);
  border-color: transparent;
  box-shadow: var(--ip-shadow-focus);
}
.ip-select--filled.ip-select--disabled .ip-select__trigger {
  background: var(--ip-gray-200);
  border-color: transparent;
}

/* REQ-UI-003B：loading 态 */
.ip-select--loading .ip-select__trigger {
  cursor: wait;
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

/* REQ-UI-003D：多选 tag 列表容器 */
.ip-select__tags {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ip-spacing-1);
  flex: 1;
  min-width: 0;
  align-items: center;
}

.ip-select__tag {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  padding: 1px var(--ip-spacing-1) 1px var(--ip-spacing-2);
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-sm);
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-loose3);
  color: var(--ip-color-text-primary);
  max-width: 100%;
  min-width: 0;
}
.ip-select__tag-icon {
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
}
.ip-select__tag-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.ip-select__tag-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-full);
  color: var(--ip-color-icon-muted);
  cursor: pointer;
  padding: 0;
  transition:
    color var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-select__tag-close:hover {
  color: var(--ip-color-text-body);
  background: var(--ip-color-bg-secondary);
}
.ip-select__tag-close:focus-visible {
  outline: 2px solid var(--ip-color-border-focus);
  outline-offset: 1px;
}
.ip-select__tag--disabled {
  opacity: 0.6;
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
 * Search（REQ-UI-003E）
 * ============================================================ */
.ip-select__search {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  margin-bottom: var(--ip-spacing-1);
  border-bottom: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
  border-top-left-radius: var(--ip-radius-md);
  border-top-right-radius: var(--ip-radius-md);
  position: sticky;
  top: 0;
  z-index: 1;
}
.ip-select__search-icon {
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
}
.ip-select__search-input {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  outline: none;
  font-family: inherit;
  font-size: inherit;
  color: var(--ip-color-text-primary);
  padding: 0;
}
.ip-select__search-input::placeholder {
  color: var(--ip-color-text-placeholder);
}
.ip-select__search-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-full);
  color: var(--ip-color-icon-muted);
  cursor: pointer;
  padding: 0;
  transition: color var(--ip-duration-fast) var(--ip-ease-out), background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-select__search-clear:hover {
  color: var(--ip-color-text-body);
  background: var(--ip-color-bg-tertiary);
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

/* REQ-UI-003D：多选模式 checkbox */
.ip-select__option-checkbox {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  background: var(--ip-color-bg-secondary);
  transition:
    color var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-select__option-checkbox--checked {
  color: var(--ip-color-text-on-primary, #fff);
  background: var(--ip-primary-500);
  border-color: var(--ip-primary-500);
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

/* REQ-UI-003B：popover loading spinner */
.ip-select__loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-6) var(--ip-spacing-4);
  color: var(--ip-color-icon-muted);
}
.ip-select__loading-text {
  font-size: var(--ip-text-caption-size);
}
.ip-select__spinner {
  animation: ip-spin var(--ip-duration-spinner) linear infinite;
}

/* REQ-UI-003C / REQ-UI-003E：空状态 */
.ip-select__empty {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-6) var(--ip-spacing-4);
  min-height: var(--ip-select-option-h);
}
.ip-select__empty-text {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  user-select: none;
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

/* filled 暗色模式 */
[data-theme='dark'] .ip-select--filled .ip-select__trigger {
  background: var(--ip-color-bg-tertiary);
  border-color: transparent;
}
[data-theme='dark'] .ip-select--filled.ip-select--hovered:not(.ip-select--open):not(.ip-select--disabled) .ip-select__trigger {
  background: var(--ip-color-bg-elevated);
}
</style>