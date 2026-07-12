/**
 * IpFlex — 公开 Props / 类型定义
 *
 * 规范:icepaw-layout-system.md v1.1 §2.3
 *
 * 定位:基于 CSS `display: flex` 的统一布局容器,合并 Space + Flex 两种职责。
 *      按 `separator` prop 自动切换 Gap / Separator 两种模式:
 *      - Gap 模式(默认,separator=false / undefined):不包裹子元素,纯 CSS gap
 *      - Separator 模式(separator=true/string/VNode):为相邻子元素之间插入分隔符
 *
 * 默认值(规范 §2.2 决策 2):align 始终为 CSS 原生 `stretch`(与 Naive UI n-flex 一致)
 */

import type { VNode } from 'vue'

/* ============================================================
 * 通用类型
 * ============================================================ */

/** 间距预设(规范 §1.1):xs=8 / sm=12 / md=16 / lg=24 / xl=32 */
export type SpaceSize = 'xs' | 'sm' | 'md' | 'lg' | 'xl' | number

/** size prop 的二元组形式(规范 §1.2):[rowGap, colGap] */
export type SizeProp = SpaceSize | [SpaceSize, SpaceSize]

/** flex-direction(规范 §2.3):采用 CSS 原生命名 */
export type FlexDirection =
  | 'row'
  | 'row-reverse'
  | 'column'
  | 'column-reverse'

/** 交叉轴对齐预设(规范 §2.3):`start` 等别名映射到 CSS `flex-start` 等 */
export type FlexAlign =
  | 'start'
  | 'center'
  | 'end'
  | 'baseline'
  | 'stretch'
  | (string & {})

/** 主轴对齐预设(规范 §2.3) */
export type FlexJustify =
  | 'start'
  | 'center'
  | 'end'
  | 'space-between'
  | 'space-around'
  | 'space-evenly'
  | (string & {})

/** flex-wrap 形式(规范 §2.3) */
export type FlexWrap = boolean | 'nowrap' | 'wrap' | 'wrap-reverse'

/** 分隔符(规范 §2.3,§2.5):true=默认线;string=居中文本;VNode=自定义 */
export type FlexSeparator = boolean | string | VNode

/* ============================================================
 * Props
 * ============================================================ */

export interface FlexProps {
  /**
   * flex-direction(规范 §2.3)
   * @default 'row'
   */
  direction?: FlexDirection

  /**
   * 便捷别名:true → direction='column'(在 direction 显式未设置时生效;
   *                                与 direction 同时设置时后者优先)
   * @default false
   */
  vertical?: boolean

  /**
   * 交叉轴对齐(规范 §2.3)
   * 预设有别名映射(start → flex-start),也接受任意 CSS align-items 值
   * @default undefined(回退到 CSS 原生 stretch)
   */
  align?: FlexAlign

  /**
   * 主轴对齐(规范 §2.3)
   * 预设有别名映射,接受任意 CSS justify-content 值
   * @default 'start'
   */
  justify?: FlexJustify

  /**
   * gap 大小(规范 §1.1,§2.3):沿用 Naive UI n-flex 的 size 命名
   * 接受预设/数字/任意 CSS 字符串;二元组形式分别控制 row/col gap
   * @default 'md'
   */
  size?: SizeProp

  /** 单独覆盖 row gap(优先级高于 size 数组的第 0 项) */
  rowGap?: SpaceSize

  /** 单独覆盖 column gap(优先级高于 size 数组的第 1 项) */
  colGap?: SpaceSize

  /**
   * flex-wrap(规范 §2.3)
   * boolean 时 true → 'wrap', false → 'nowrap'
   * @default false
   */
  wrap?: FlexWrap

  /**
   * 是否为 inline-flex 容器(规范 §2.3)
   * @default false
   */
  inline?: boolean

  /**
   * 快速切换方向:在 direction 基础上叠加 *-reverse 后缀(规范 §2.3)
   * 优先级 reverse > vertical > direction
   * @default false
   */
  reverse?: boolean

  /**
   * 分隔符(规范 §2.3,§2.5)
   * - true:默认竖线(行)/ 横线(列)
   * - string:居中文本
   * - VNode:自定义 slot / 节点
   * 启用后切换到 Separator 模式(遍历子节点 + 插入分隔符)
   * @default false
   */
  separator?: FlexSeparator

  /**
   * 自身 `flex` 属性(规范 §2.3)
   * 数字简写:`flex=1` → `flex: 1 1 0%`
   * 字符串原样使用
   */
  flex?: string | number

  /**
   * 自定义根元素标签(规范 §2.3)
   * @default 'div'
   */
  as?: string
}
