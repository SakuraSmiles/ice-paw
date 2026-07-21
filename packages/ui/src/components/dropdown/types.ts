/**
 * DropdownMenu — 公开 Props / Emits
 *
 * 规范：icepaw-p0-component-specs.md §五
 */

import type { Component } from 'vue'

export type DropdownPlacement = 'bottom-start' | 'bottom-end' | 'top-start' | 'top-end'

export type DropdownItem =
  | {
      type?: 'item'
      key?: string
      label: string
      icon?: Component
      shortcut?: string
      disabled?: boolean
      danger?: boolean
      /** 选中后是否保留菜单（默认 false：选中后关闭） */
      keepOpen?: boolean
      onClick?: () => void | Promise<void>
    }
  | { type: 'divider'; key?: string }
  | { type: 'label'; text: string; key?: string }

export interface DropdownProps {
  /** 受控显隐（必填） */
  modelValue: boolean

  /** 菜单项列表 */
  items: DropdownItem[]

  /** 浮层位置。默认 'bottom-end' */
  placement?: DropdownPlacement

  /** 浮层宽度。默认 200px */
  width?: number | string

  /** 自定义触发器 slot 名（默认 'trigger'） */
  trigger?: string

  /** 触发方式：click（默认） / hover（仅桌面端） */
  triggerAction?: 'click' | 'hover'

  /** hover 触发时进入触发器延迟（ms），避免误触。默认 100ms */
  hoverDelay?: number

  /** 自定义根节点 aria-label（trigger 自身的） */
  ariaLabel?: string
}

export interface DropdownEmits {
  (e: 'update:modelValue', value: boolean): void
  (e: 'select', item: DropdownItem, index: number): void
  (e: 'open'): void
  (e: 'close'): void
}