/**
 * EmptyState — 公开 Props / Emits
 *
 * 规范：icepaw-p0-component-specs.md §四
 */

import type { Component } from 'vue'

export type EmptyStateIconSize = 'sm' | 'md' | 'lg' | 'xl' | '2xl' | '3xl'

export interface EmptyStateAction {
  label: string
  onClick?: (ev: MouseEvent) => void | Promise<void>
  icon?: Component
  /** 危险样式（红），仅 secondaryAction 使用，primaryAction 忽略 */
  danger?: boolean
}

export interface EmptyStateProps {
  /** 主图标（Lucide Component） */
  icon?: Component

  /** 图标尺寸梯度。默认 'xl' (48px) */
  iconSize?: EmptyStateIconSize

  /** 标题（必填） */
  title: string

  /** 描述（可选） */
  description?: string

  /** 主 CTA（IpButton 实例也可；本字段为内置便捷用法） */
  primaryAction?: EmptyStateAction

  /** 次 CTA */
  secondaryAction?: EmptyStateAction

  /** 居中布局。默认 true；卡片内嵌时设 false 左对齐 */
  centered?: boolean

  /** 紧凑模式（小卡片内使用，视觉密度下降 30%） */
  compact?: boolean

  /** 自定义根节点 aria-label（默认基于 title 推断） */
  ariaLabel?: string
}

export interface EmptyStateEmits {
  /** primaryAction 点击（与 primaryAction.onClick 二选一，优先用 prop） */
  (e: 'primary', ev: MouseEvent): void
  /** secondaryAction 点击 */
  (e: 'secondary', ev: MouseEvent): void
}