/**
 * Select — 公开 Props / Emits
 *
 * 规范：icepaw-p0-component-specs.md §三
 * 定位：受控下拉选择，支持单选 / 多选 / 搜索过滤
 */

import type { Component } from 'vue'

export type SelectSize = 'sm' | 'md' | 'lg'

/** REQ-UI-003：Select 视觉变体 */
export type SelectVariant = 'outline' | 'filled'

export interface SelectOption<V extends string | number = string> {
  /** 选项值（必填） */
  value: V
  /** 显示文本（必填） */
  label: string
  /** 禁用 */
  disabled?: boolean
  /** 选项前导图标（Lucide Component） */
  icon?: Component
  /** 选项描述（hover tooltip 或副标题） */
  description?: string
}

/**
 * Select v-model 类型
 *  - 单选（multiple=false，默认）：`V | null`
 *  - 多选（multiple=true）：`V[]`
 *
 * 注：受控组件的值由父级管理，组件本身不区分两种形态；
 * 类型由 `multiple` prop 在使用处由调用方通过 generic 推断。
 */
export type SelectModelValue<V extends string | number = string> = V | null | V[]

export interface SelectProps<V extends string | number = string> {
  /**
   * v-model 绑定值。
   * - 单选模式（multiple=false）：`V | null`
   * - 多选模式（multiple=true）：`V[]`
   *
   * 类型上保持兼容（`V | null | V[]`），由调用方按 `multiple` 自行保证。
   */
  modelValue?: V | null | V[]

  /** 选项列表 */
  options: SelectOption<V>[]

  /** 占位符（modelValue 为空时显示） */
  placeholder?: string

  /** 尺寸。默认 'md'（与 Input 完全对齐） */
  size?: SelectSize

  /** REQ-UI-003：视觉变体（outline 边框态 / filled 填充态） */
  variant?: SelectVariant

  /** 禁用 */
  disabled?: boolean

  /** 错误态（红边 + danger focus ring） */
  error?: boolean

  /** 错误信息 */
  errorMessage?: string

  /** 是否可清空（modelValue 非空时显示 ✕） */
  clearable?: boolean

  /** 触发器前导图标（Lucide Component） */
  prefixIcon?: Component

  /** 浮层 placement，默认 'bottom-start' */
  placement?: 'bottom-start' | 'bottom-end' | 'top-start' | 'top-end'

  /** 浮层宽度：'match-trigger'（默认，与触发器等宽） / 'auto'（按内容） / 数字 / CSS 字符串 */
  popoverWidth?: 'match-trigger' | 'auto' | number | string

  /** 自定义根节点 aria-label */
  ariaLabel?: string

  /** REQ-UI-003B：加载态，浮层内显示 spinner，禁用选项交互 */
  loading?: boolean

  /** 关联 label id（a11y） */
  id?: string

  /**
   * REQ-UI-003D：多选模式
   * - true：trigger 内显示 tag 列表，浮层内选项前显示 checkbox，点击不自动关闭浮层
   * - false（默认）：单选，点击选项后自动关闭浮层
   */
  multiple?: boolean

  /**
   * REQ-UI-003E：搜索过滤
   * - true：浮层顶部显示搜索输入框（自动聚焦），按 `option.label.includes(search)` 实时过滤
   * - false（默认）：不显示搜索框
   */
  filterable?: boolean

  /**
   * 自定义过滤函数（filterable=true 时生效）。
   * 默认：`label.toLowerCase().includes(searchValue.toLowerCase())`
   * 返回 true 表示该选项命中。
   */
  filter?: (searchValue: string, option: SelectOption<V>) => boolean

  /** REQ-UI-003E：搜索框 placeholder */
  searchPlaceholder?: string

  /** REQ-UI-003C：options 为空时的占位文案 */
  emptyText?: string

  /** REQ-UI-003E：搜索无匹配时的占位文案 */
  noMatchText?: string
}

export interface SelectEmits<V extends string | number = string> {
  /**
   * v-model 更新。
   * - 单选：`V | null`
   * - 多选：`V[]`
   */
  (e: 'update:modelValue', value: V | null | V[]): void
  /** change 事件（单选：当前值；多选：当前值数组） */
  (e: 'change', value: V | V[]): void
  (e: 'clear'): void
  (e: 'open'): void
  (e: 'close'): void
  /** REQ-UI-003D：多选模式下 tag × 取消选中时 */
  (e: 'remove-tag', value: V): void
  /** REQ-UI-003E：搜索值变化时 */
  (e: 'search', value: string): void
}