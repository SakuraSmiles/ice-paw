/**
 * Popconfirm — 公开 Props / Emits
 *
 * 规范：icepaw-p0-component-specs.md §六
 * 定位：轻量气泡式确认（替代 window.confirm）
 */

export type PopconfirmPlacement = 'top' | 'bottom' | 'left' | 'right'
export type PopconfirmTrigger = 'click' | 'hover'

export interface PopconfirmProps {
  /** 受控显隐（必填） */
  modelValue: boolean

  /** 标题（必填） */
  title: string

  /** 描述（可选） */
  description?: string

  /** 确认按钮文字。默认 '确认' */
  confirmText?: string

  /** 取消按钮文字。默认 '取消' */
  cancelText?: string

  /** 危险样式（红色 confirm 按钮） */
  danger?: boolean

  /** 触发方式：click（默认） / hover */
  trigger?: PopconfirmTrigger

  /** 浮层位置。默认 'top' */
  placement?: PopconfirmPlacement

  /** 确认按钮 loading 态（异步删除等场景） */
  loading?: boolean

  /** 触发器自定义 slot 名（默认 'trigger'） */
  triggerSlot?: string

  /** 浮层宽度（CSS 值）。默认 'auto' */
  width?: number | string

  /** 自定义根节点 aria-label */
  ariaLabel?: string
}

export interface PopconfirmEmits {
  (e: 'update:modelValue', value: boolean): void
  (e: 'confirm'): void
  (e: 'cancel'): void
  (e: 'open'): void
  (e: 'close'): void
}