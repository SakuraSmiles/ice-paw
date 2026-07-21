/**
 * Select — 公开 Props / Emits
 *
 * 规范：icepaw-p0-component-specs.md §三
 * 定位：受控下拉选择，单选场景
 */

import type { Component } from 'vue'

export type SelectSize = 'sm' | 'md' | 'lg'

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

export interface SelectProps<V extends string | number = string> {
  /** v-model 绑定值 */
  modelValue?: V | null

  /** 选项列表 */
  options: SelectOption<V>[]

  /** 占位符（modelValue 为空时显示） */
  placeholder?: string

  /** 尺寸。默认 'md'（与 Input 完全对齐） */
  size?: SelectSize

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

  /** 关联 label id（a11y） */
  id?: string
}

export interface SelectEmits<V extends string | number = string> {
  (e: 'update:modelValue', value: V | null): void
  (e: 'change', value: V): void
  (e: 'clear'): void
  (e: 'open'): void
  (e: 'close'): void
}