<script setup lang="ts">
/**
 * IpIconPicker — IcePaw 图标选择器
 *
 * 从 Lucide 图标集中搜索 / 分类浏览选择图标。
 * 选中后输出图标名（字符串）。
 *
 * a11y：搜索框 aria-label, 列表 role="listbox", 选项 role="option"
 */
import { computed, ref, watch, nextTick, type Component } from 'vue'
import { Search, X } from 'lucide-vue-next'
import { icons } from 'lucide-vue-next'
import type { IconPickerProps, IconPickerEmits } from './types'

const props = withDefaults(defineProps<IconPickerProps>(), {
  modelValue: null,
  categories: () => [],
  searchPlaceholder: '搜索图标...',
  disabled: false,
  pageSize: 24,
})

const emit = defineEmits<IconPickerEmits>()

/* ----- Lucide 图标注册表 ----- */
const lucideIcons = icons as Record<string, Component>

/* ----- 状态 ----- */
const searchQuery = ref('')
const activeCategory = ref<string | null>(null)
const scrollContainerRef = ref<HTMLElement | null>(null)
const searchInputRef = ref<HTMLInputElement | null>(null)
const focusedIndex = ref(-1)
const COLS = 8 // 固定列数，与 grid-template-columns 配合

/* ----- 分页（v4 REQ-UI-009A）----- */
const PAGE_SIZE_DEFAULT = 24
const pageSize = computed<number>(() => Math.max(1, props.pageSize ?? PAGE_SIZE_DEFAULT))
const currentPage = ref(1) // 当前页码（1-indexed）
// 防止 vue-tsc 静态分析误报（pageSize / currentPage 在 watch / computed / template 中均被引用）
void pageSize.value
void currentPage.value

/* ----- 全量图标列表（过滤掉内部 key） ----- */
const allIconNames = computed(() => {
  return Object.keys(lucideIcons).filter(
    (name) => /^[A-Z][a-zA-Z0-9]*$/.test(name),
  )
})

/* ----- 当前分类的图标名列表 ----- */
const activeIcons = computed(() => {
  if (activeCategory.value) {
    const cat = props.categories.find((c) => c.id === activeCategory.value)
    if (cat) return cat.icons.filter((n) => n in lucideIcons)
  }
  return allIconNames.value
})

/* ----- 搜索过滤（全量，分页前）----- */
const filteredAllIcons = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return activeIcons.value
  return activeIcons.value.filter((name) => name.toLowerCase().includes(q))
})

/* ----- 分页计算（v4 REQ-UI-009A）----- */
const totalPages = computed<number>(() => {
  return Math.max(1, Math.ceil(filteredAllIcons.value.length / pageSize.value))
})

/** 超出总页数时自动钳位 */
watch([filteredAllIcons, pageSize], () => {
  if (currentPage.value > totalPages.value) {
    currentPage.value = totalPages.value
  }
}, { immediate: true })

/** 当前页可见图标列表 */
const filteredIcons = computed(() => {
  const start = (currentPage.value - 1) * pageSize.value
  return filteredAllIcons.value.slice(start, start + pageSize.value)
})

/** 总图数过少时（不足一页）不显示分页 */
const showPagination = computed<boolean>(() => filteredAllIcons.value.length > pageSize.value)

/* ----- 派生分类列表（自动推断 categories 为空时） ----- */
const displayCategories = computed(() => {
  if (props.categories.length > 0) return props.categories
  // 无分类时展示 "全部"
  return [{ id: '__all__', label: '全部', icons: allIconNames.value }]
})

/* ----- 选中 ----- */
function selectIcon(name: string): void {
  if (props.disabled) return
  if (props.modelValue === name) {
    emit('update:modelValue', null)
  } else {
    emit('update:modelValue', name)
  }
}

/* ----- 分类切换 ----- */
function switchCategory(catId: string): void {
  activeCategory.value = catId === '__all__' ? null : catId
  searchQuery.value = ''
  currentPage.value = 1 /* REQ-UI-009A：切换分类回到第 1 页 */
  nextTick(() => scrollContainerRef.value?.scrollTo({ top: 0 }))
}

/* ----- 搜索清除 ----- */
function clearSearch(): void {
  searchQuery.value = ''
  searchInputRef.value?.focus()
}

/* ----- 图标组件动态解析 ----- */
function resolveIcon(name: string): Component | undefined {
  return lucideIcons[name]
}

/* ----- 点击外部不清除（popover 场景由外部处理） ----- */

/* ----- 重置搜索当 categories 变化 ----- */
watch(
  () => props.categories,
  () => {
    activeCategory.value = null
    searchQuery.value = ''
    currentPage.value = 1
  },
)

/* 搜索/分类变化时回到第 1 页 */
watch([searchQuery, activeCategory], () => {
  currentPage.value = 1
})

/* 过滤变化时重置焦点 */
watch(filteredAllIcons, () => {
  focusedIndex.value = -1
})

/* ----- 分页交互（REQ-UI-009A）----- */
function goPrev(): void {
  if (currentPage.value <= 1) return
  currentPage.value -= 1
  nextTick(() => scrollContainerRef.value?.scrollTo({ top: 0 }))
}

function goNext(): void {
  if (currentPage.value >= totalPages.value) return
  currentPage.value += 1
  nextTick(() => scrollContainerRef.value?.scrollTo({ top: 0 }))
}

function goToPage(p: number): void {
  if (p < 1 || p > totalPages.value || p === currentPage.value) return
  currentPage.value = p
  nextTick(() => scrollContainerRef.value?.scrollTo({ top: 0 }))
}

/** 可见页码省略号列表 */
const visiblePages = computed<Array<number | 'ellipsis'>>(() => {
  const total = totalPages.value
  const cur = currentPage.value
  if (total <= 7) {
    return Array.from({ length: total }, (_, i) => i + 1)
  }
  const out: Array<number | 'ellipsis'> = []
  const add = (v: number | 'ellipsis') => out.push(v)
  add(1)
  if (cur > 4) add('ellipsis')
  const start = Math.max(2, cur - 2)
  const end = Math.min(total - 1, cur + 2)
  for (let i = start; i <= end; i++) add(i)
  if (cur < total - 3) add('ellipsis')
  add(total)
  return out
})

/* ----- 键盘导航（WAI-ARIA listbox aria-activedescendant） ----- */
function gridItemId(index: number): string {
  return `ip-icon-${filteredIcons.value[index]}`
}

function scrollToFocused(): void {
  const container = scrollContainerRef.value
  if (!container || focusedIndex.value < 0) return
  const id = gridItemId(focusedIndex.value)
  const el = container.querySelector(`#${id}`) as HTMLElement | null
  el?.scrollIntoView({ block: 'nearest' })
}

function handleGridKeydown(e: KeyboardEvent): void {
  const total = filteredIcons.value.length
  if (total === 0) return

  let idx = focusedIndex.value

  // 首次进入网格（Tab 进来）
  if (idx < 0) {
    if (['ArrowRight', 'ArrowLeft', 'ArrowDown', 'ArrowUp', 'Home', 'End', 'PageUp', 'PageDown'].includes(e.key)) {
      idx = 0
    } else {
      return
    }
  }

  const col = idx % COLS
  const row = Math.floor(idx / COLS)

  switch (e.key) {
    case 'ArrowRight':
      if (idx < total - 1) idx++
      break
    case 'ArrowLeft':
      if (idx > 0) idx--
      break
    case 'ArrowDown': {
      const next = idx + COLS
      idx = next < total ? next : total - 1
      break
    }
    case 'ArrowUp': {
      const prev = idx - COLS
      idx = prev >= 0 ? prev : col
      break
    }
    case 'Home':
      idx = row * COLS
      break
    case 'End': {
      const rowEnd = Math.min((row + 1) * COLS, total) - 1
      idx = rowEnd
      break
    }
    case 'PageDown': {
      // 向下移动可见行数（约 7 行，每行 COLS）
      const pageRows = 7
      const nextIdx = idx + pageRows * COLS
      idx = nextIdx < total ? nextIdx : total - 1
      break
    }
    case 'PageUp': {
      const pageRows = 7
      const prevIdx = idx - pageRows * COLS
      idx = prevIdx >= 0 ? prevIdx : row * COLS
      break
    }
    case 'Enter':
    case ' ':
      e.preventDefault()
      selectIcon(filteredIcons.value[idx])
      return
    case 'Escape':
      searchInputRef.value?.focus()
      focusedIndex.value = -1
      return
    default:
      return
  }

  e.preventDefault()
  focusedIndex.value = idx
  nextTick(scrollToFocused)
}
</script>

<template>
  <div :class="['ip-icon-picker', { 'ip-icon-picker--disabled': disabled }]">
    <!-- 搜索栏 -->
    <div class="ip-icon-picker__search">
      <Search :size="16" class="ip-icon-picker__search-icon" aria-hidden="true" />
      <input
        ref="searchInputRef"
        v-model="searchQuery"
        type="text"
        class="ip-icon-picker__search-input"
        :placeholder="searchPlaceholder"
        :disabled="disabled"
        aria-label="搜索图标"
        @keydown.stop
      />
      <button
        v-if="searchQuery"
        type="button"
        class="ip-icon-picker__search-clear"
        :disabled="disabled"
        aria-label="清除搜索"
        @click="clearSearch"
      >
        <X :size="14" />
      </button>
    </div>

    <!-- 分类 Tab 栏 -->
    <div v-if="displayCategories.length > 1" class="ip-icon-picker__categories" role="tablist">
      <button
        v-for="cat in displayCategories"
        :key="cat.id"
        type="button"
        :class="[
          'ip-icon-picker__category',
          {
            'ip-icon-picker__category--active':
              (cat.id === '__all__' && !activeCategory) || cat.id === activeCategory,
          },
        ]"
        role="tab"
        :aria-selected="(cat.id === '__all__' && !activeCategory) || cat.id === activeCategory"
        :disabled="disabled"
        @click="switchCategory(cat.id)"
      >
        {{ cat.label }}
      </button>
    </div>

    <!-- 图标网格 -->
    <div
      ref="scrollContainerRef"
      class="ip-icon-picker__grid"
      tabindex="0"
      role="grid"
      :aria-label="'图标列表 共 ' + filteredAllIcons.length + ' 个，当前第 ' + currentPage + ' / ' + totalPages + ' 页'"
      :aria-activedescendant="focusedIndex >= 0 ? gridItemId(focusedIndex) : undefined"
      @keydown="handleGridKeydown"
    >
      <button
        v-for="(name, index) in filteredIcons"
        :id="gridItemId(index)"
        :key="name"
        type="button"
        :class="[
          'ip-icon-picker__item',
          {
            'ip-icon-picker__item--selected': modelValue === name,
            'ip-icon-picker__item--focused': focusedIndex === index,
          },
        ]"
        role="gridcell"
        :aria-selected="modelValue === name"
        :aria-label="name"
        :disabled="disabled"
        :title="name"
        @click="selectIcon(name)"
      >
        <component
          :is="resolveIcon(name)"
          :size="20"
          :stroke-width="1.75"
          aria-hidden="true"
        />
      </button>

      <!-- 空状态 -->
      <div v-if="filteredIcons.length === 0" class="ip-icon-picker__empty">
        未找到匹配图标
      </div>
    </div>

    <!-- REQ-UI-009A：分页控件 -->
    <nav
      v-if="showPagination"
      class="ip-icon-picker__pagination"
      role="navigation"
      aria-label="图标分页"
    >
      <button
        type="button"
        class="ip-icon-picker__page-btn"
        :disabled="disabled || currentPage <= 1"
        aria-label="上一页"
        @click="goPrev"
      >‹</button>

      <template v-for="(item, i) in visiblePages" :key="`p-${i}`">
        <button
          v-if="item !== 'ellipsis'"
          type="button"
          :class="[
            'ip-icon-picker__page-btn',
            'ip-icon-picker__page-btn--num',
            { 'ip-icon-picker__page-btn--active': item === currentPage },
          ]"
          :disabled="disabled"
          :aria-current="item === currentPage ? 'page' : undefined"
          :aria-label="`第 ${item} 页 / 共 ${totalPages} 页`"
          @click="goToPage(item as number)"
        >{{ item }}</button>
        <span v-else class="ip-icon-picker__page-ellipsis" aria-hidden="true">…</span>
      </template>

      <button
        type="button"
        class="ip-icon-picker__page-btn"
        :disabled="disabled || currentPage >= totalPages"
        aria-label="下一页"
        @click="goNext"
      >›</button>

      <span class="ip-icon-picker__page-info" aria-live="polite">
        {{ filteredAllIcons.length }} 个 · 第 {{ currentPage }} / {{ totalPages }} 页
      </span>
    </nav>
  </div>
</template>

<style scoped>
/* ============================================================
 * IpIconPicker — 图标选择器
 * ============================================================ */
.ip-icon-picker {
  display: flex;
  flex-direction: column;
  gap: 0;
  width: 100%;
  max-width: 320px;
  font-family: var(--ip-font-sans);
  color: var(--ip-color-text-primary);
}

.ip-icon-picker--disabled {
  opacity: 0.5;
  pointer-events: none;
}

/* ----- 搜索栏 ----- */
.ip-icon-picker__search {
  position: relative;
  display: flex;
  align-items: center;
  padding: 0 var(--ip-spacing-3);
  height: var(--ip-input-h-sm);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-input-radius) var(--ip-input-radius) 0 0;
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-icon-picker__search:focus-within {
  border-color: var(--ip-color-border-focus);
}

.ip-icon-picker__search-icon {
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
  margin-right: var(--ip-spacing-2);
}

.ip-icon-picker__search-input {
  flex: 1;
  min-width: 0;
  height: 100%;
  padding: 0;
  font-size: var(--ip-text-body-sm-size);
  font-family: inherit;
  line-height: 1;
  color: var(--ip-color-text-primary);
  background: transparent;
  border: none;
  outline: none;
}

.ip-icon-picker__search-input::placeholder {
  color: var(--ip-color-text-placeholder);
}

.ip-icon-picker__search-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  margin-left: var(--ip-spacing-1);
  color: var(--ip-color-icon-muted);
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  flex-shrink: 0;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-icon-picker__search-clear:hover {
  color: var(--ip-color-icon-default);
}

/* ----- 分类 Tab ----- */
.ip-icon-picker__categories {
  display: flex;
  align-items: stretch;
  overflow-x: auto;
  border-bottom: 1px solid var(--ip-color-border-default);
  border-left: 1px solid var(--ip-color-border-default);
  border-right: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
  scrollbar-width: none;
}

.ip-icon-picker__categories::-webkit-scrollbar {
  display: none;
}

.ip-icon-picker__category {
  position: relative;
  display: inline-flex;
  align-items: center;
  height: 32px;
  padding: 0 var(--ip-spacing-3);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  font-family: inherit;
  color: var(--ip-color-text-tertiary);
  background: transparent;
  border: none;
  border-radius: 0;
  cursor: pointer;
  white-space: nowrap;
  line-height: 1;
  flex-shrink: 0;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-icon-picker__category:hover {
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
}

.ip-icon-picker__category--active {
  color: var(--ip-primary-600);
}

.ip-icon-picker__category--active::after {
  content: '';
  position: absolute;
  bottom: 0;
  left: var(--ip-spacing-3);
  right: var(--ip-spacing-3);
  height: 2px;
  background: var(--ip-primary-600);
  border-radius: 1px 1px 0 0;
}

.ip-icon-picker__category:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ----- 图标网格 ----- */
.ip-icon-picker__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(36px, 1fr));
  gap: var(--ip-spacing-1);
  padding: var(--ip-spacing-3);
  max-height: 320px;
  overflow-y: auto;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-top: none;
  border-radius: 0 0 var(--ip-input-radius) var(--ip-input-radius);
}

/* 滚动条 */
.ip-icon-picker__grid::-webkit-scrollbar {
  width: 6px;
}
.ip-icon-picker__grid::-webkit-scrollbar-track {
  background: transparent;
}
.ip-icon-picker__grid::-webkit-scrollbar-thumb {
  background: var(--ip-color-bg-tertiary);
  border-radius: 3px;
}

/* ----- 焦态（aria-activedescendant 视觉提示） ----- */
.ip-icon-picker__item--focused {
  background: var(--ip-color-bg-tertiary);
  outline: 2px solid var(--ip-primary-500);
  outline-offset: -2px;
}

/* ----- 图标项 ----- */
.ip-icon-picker__item {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  padding: 0;
  color: var(--ip-color-icon-default);
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    border-color     var(--ip-duration-fast) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-icon-picker__item:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-default);
}

.ip-icon-picker__item:active {
  transform: scale(0.92);
}

.ip-icon-picker__item:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.ip-icon-picker__item--selected {
  background: color-mix(in srgb, var(--ip-primary-500) 12%, transparent);
  border-color: var(--ip-primary-500);
  color: var(--ip-primary-500);
}

.ip-icon-picker__item--selected:hover {
  background: color-mix(in srgb, var(--ip-primary-500) 18%, transparent);
  border-color: var(--ip-primary-500);
}

/* ----- 空状态 ----- */
.ip-icon-picker__empty {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-6);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

/* ============================================================
 * 分页控件（v4 REQ-UI-009A）
 * ============================================================ */
.ip-icon-picker__pagination {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--ip-spacing-1);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-top: none;
  border-radius: 0 0 var(--ip-input-radius) var(--ip-input-radius);
  flex-wrap: wrap;
}
.ip-icon-picker__page-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 28px;
  height: 28px;
  padding: 0 var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  font-family: inherit;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  line-height: 1;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out),
    border-color var(--ip-duration-fast) var(--ip-ease-out),
    color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-icon-picker__page-btn:hover:not(:disabled) {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-default);
  color: var(--ip-color-text-primary);
}
.ip-icon-picker__page-btn:active:not(:disabled) { transform: scale(0.94); }
.ip-icon-picker__page-btn:focus-visible { outline: none; box-shadow: var(--ip-shadow-focus); }
.ip-icon-picker__page-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.ip-icon-picker__page-btn--num { font-variant-numeric: tabular-nums; }
.ip-icon-picker__page-btn--active,
.ip-icon-picker__page-btn--active:hover {
  background: var(--ip-primary-500);
  border-color: var(--ip-primary-500);
  color: var(--ip-color-text-on-primary, #fff);
}
.ip-icon-picker__page-ellipsis {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 20px;
  height: 28px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  user-select: none;
}
.ip-icon-picker__page-info {
  margin-left: var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
