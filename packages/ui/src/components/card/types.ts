/**
 * Card — 公开 Props / Emits
 *
 * 规范：icepaw-p0-component-specs.md §二
 */

import type { SpaceSize } from '../flex/types'

/** 视觉变体 */
export type CardVariant = 'bordered' | 'filled' | 'shadow'

/** 内边距梯度：none=0 / sm=12 / md=16 / lg=24，或复用 SpaceSize / 任意 CSS 值 */
export type CardPadding = 'none' | 'sm' | 'md' | 'lg' | SpaceSize

/** 渲染元素：div / button / a */
export type CardAs = 'div' | 'button' | 'a'

export interface CardProps {
  /** 视觉变体：bordered（默认，内敛） / filled（实色填充） / shadow（浮起） */
  variant?: CardVariant

  /** 内边距梯度。默认 'md' (16px)。也可传 SpaceSize / 数字 / 字符串（复用 space-size 解析） */
  padding?: CardPadding | number | string

  /** 交互态：true 时 hover 抬升 + cursor pointer + 支持点击事件 */
  interactive?: boolean

  /** 选中态：描边变 primary + 浅蓝底色（用于项目当前选中 / 多选场景） */
  selected?: boolean

  /** 渲染元素。默认 'div'。as='button' 时键盘可达 + Enter/Space 触发 click */
  as?: CardAs

  /** 禁用（仅 as='button' 生效；灰态 + pointer-events: none） */
  disabled?: boolean

  /** as='a' 时使用（渲染为 <a :href="href">） */
  href?: string

  /** as='a' 时 target 属性 */
  target?: '_blank' | '_self' | '_parent' | '_top'

  /** 块级（width: 100%） */
  block?: boolean

  /** 自定义根节点 aria-label */
  ariaLabel?: string
}

export interface CardEmits {
  /** 点击事件（interactive && !disabled 时触发） */
  (e: 'click', ev: MouseEvent): void

  /** 双击（仅 interactive） */
  (e: 'dblclick', ev: MouseEvent): void
}