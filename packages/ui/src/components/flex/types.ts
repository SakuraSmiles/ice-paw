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
 *
 * REQ-UI-004:`size` prop 增加 `gap` 别名(语义更明确),两者等价,优先取 `gap`
 * REQ-UI-004A:`breakpoints` prop + ResizeObserver 实现响应式断点
 */

import type { VNode } from 'vue'

/* ============================================================
 * 通用类型
 * ============================================================ */

/** 间距预设(规范 §1.1):xs=8 / sm=12 / md=16 / lg=24 / xl=32 */
export type SpaceSize = 'xs' | 'sm' | 'md' | 'lg' | 'xl' | number

/** size / gap prop 的二元组形式(规范 §1.2):[rowGap, colGap] */
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
 * REQ-UI-004A：响应式断点(breakpoints)
 *  - 单个断点定义:触发该断点时的覆盖配置
 *  - `width`:触发断点的容器最小宽度(像素)
 *  - 容器宽度 >= 该值时激活
 * ============================================================ */

export interface FlexBreakpoint {
  /** 触发该断点的容器宽度阈值(px);容器宽度 >= width 时该断点生效 */
  width?: number
  /** 覆盖 direction(可选) */
  direction?: FlexDirection
  /** 覆盖 align(可选) */
  align?: FlexAlign
  /** 覆盖 justify(可选) */
  justify?: FlexJustify
  /** 覆盖 gap(可选);支持二元组 [row, col] */
  gap?: SizeProp
  /** 单独覆盖 row gap */
  rowGap?: SpaceSize
  /** 单独覆盖 column gap */
  colGap?: SpaceSize
  /** 覆盖 wrap */
  wrap?: FlexWrap
  /** 覆盖 inline */
  inline?: boolean
  /** 覆盖 reverse */
  reverse?: boolean
}

/**
 * breakpoints prop 的整体形态:
 *   { mobile: { width: 640, direction: 'column', gap: 8 }, desktop: { width: 1024 } }
 *
 * 注:键名任意(作为 ID 用于断点排序);激活规则为"宽度最大且 <= 容器宽度的断点"
 */
export type FlexBreakpoints = Record<string, FlexBreakpoint>

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
   *
   * REQ-UI-004:`gap` 与 `size` 等价;同时传入时 `gap` 优先。
   */
  size?: SizeProp

  /**
   * REQ-UI-004:`gap` 是 `size` 的语义别名(推荐命名);
   * 与 `size` 同时存在时优先;若非法值(非 SpaceSize / 非二元组),console.warn 并忽略。
   * 单独覆盖 row gap(优先级高于 size 数组的第 0 项)
   */
  gap?: SizeProp

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

  /**
   * REQ-UI-004A:响应式断点
   *
   *   {
   *     mobile: { width: 640, direction: 'column', gap: 8 },
   *     tablet: { width: 768 },
   *     desktop: { width: 1024, direction: 'row', gap: 16 },
   *   }
   *
   *  - key:任意字符串 ID;内部按 `width` 升序排序
   *  - 激活规则:"容器宽度 >= 断点 width"的最大 width 断点生效;
   *    若均不满足则使用基础 props
   *  - 断点内字段为可选;未提供时使用基础 props
   *  - 通过 ResizeObserver 监听容器尺寸变化
   */
  breakpoints?: FlexBreakpoints
}