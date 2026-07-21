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
})

const emit = defineEmits<IconPickerEmits>()

/* ----- Lucide 图标注册表 ----- */
const lucideIcons = icons as Record<string, Component>

/* ----- 状态 ----- */
const searchQuery = ref('')
const activeCategory = ref<string | null>(null)
const scrollContainerRef = ref<HTMLElement | null>(null)
const searchInputRef = ref<HTMLInputElement | null>(null)

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

/* ----- 搜索过滤 ----- */
const filteredIcons = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return activeIcons.value
  return activeIcons.value.filter((name) => name.toLowerCase().includes(q))
})

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
  },
)
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
    <div ref="scrollContainerRef" class="ip-icon-picker__grid" role="listbox" aria-label="图标列表">
      <button
        v-for="name in filteredIcons"
        :key="name"
        type="button"
        :class="[
          'ip-icon-picker__item',
          { 'ip-icon-picker__item--selected': modelValue === name },
        ]"
        role="option"
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
</style>
