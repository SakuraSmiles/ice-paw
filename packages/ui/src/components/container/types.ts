/**
 * IpContainer — 公开 Props / 类型定义
 *
 * 规范:icepaw-layout-system.md v1.1 §3
 *
 * 定位:水平居中容器,控制内容区域最大宽度。
 * 5 档预设(规范 §3.4.1):
 *   sm=480  md=720(默认,与 design-system §1.3.3 消息区域一致)
 *   lg=960  xl=1200  full=100%
 *
 * paddingX / paddingY 支持 SpaceSize / boolean / number / string(规范 §3.2 + §3.3.2)
 *   - boolean: false = 0;true = 'md'(= --ip-spacing-4)
 *   - SpaceSize / number / string:复用 IpFlex 的间距解析
 */

import type { SpaceSize } from '../flex/types'

/** 5 档 maxWidth 预设 */
export type ContainerMaxWidth = 'sm' | 'md' | 'lg' | 'xl' | 'full'

/** padding 形式(规范 §3.2):boolean + SpaceSize + number + 任意 CSS 字符串 */
export type ContainerPadding = boolean | SpaceSize | number | string

/* ============================================================
 * Props
 * ============================================================ */

export interface ContainerProps {
  /**
   * 内容区域最大宽度(规范 §3.2)
   * 预设 + 数字(px) + 任意 CSS 字符串(如 '80ch' / 'var(--xxx)')
   * @default 'md'
   */
  maxWidth?: ContainerMaxWidth | number | string

  /**
   * 是否水平居中(margin-inline: auto)
   * @default true
   */
  centered?: boolean

  /**
   * 水平内边距(规范 §3.2)
   * false = 0;true = 'md'(= --ip-spacing-4)
   * 复用 IpFlex 的间距解析(预设/数字/CSS 字符串)
   * @default 'md'
   */
  paddingX?: ContainerPadding

  /**
   * 垂直内边距(规范 §3.2)
   * 默认无(保持最小高度自适应);其余语义同 paddingX
   * @default false
   */
  paddingY?: ContainerPadding

  /**
   * 自定义根元素标签(规范 §3.2:Container 用 `tag`,与 v1.0 IpFlex 用 `as` 区分)
   * @default 'div'
   */
  tag?: string

  /**
   * 强制 width: 100%,忽略 maxWidth(规范 §3.2)
   * @default false
   */
  fluid?: boolean
}
