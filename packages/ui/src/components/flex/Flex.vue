<script setup lang="ts">
/**
 * IpFlex — IcePaw 统一布局组件
 *
 * 规范:icepaw-layout-system.md v1.1 §2
 * 定位:合并 Space + Flex 两种职责,按 `separator` prop 自动切换:
 *      - Gap 模式(默认,separator=false / undefined):零包裹,纯 CSS gap
 *      - Separator 模式(separator=true/string/VNode):遍历子节点 + 插入分隔符
 *
 * 关键设计:
 *  - 默认 align 回退到 CSS 原生 stretch(规范 §2.2 决策 2)
 *  - size 三种来源优先级(规范 §2.4.3):rowGap/colGap > size 数组 > size 单一
 *  - direction/vertical/reverse 三者优先级(规范 §2.4.5):reverse > vertical > direction
 *  - 数字 flex 简写:flex=1 → "1 1 0%"
 *  - 分隔符 aria-hidden=true(规范 §2.5.3)
 *  - REQ-UI-004:`gap` 是 `size` 的别名,优先 `gap`;非法值 console.warn
 *  - REQ-UI-004A:`breakpoints` + ResizeObserver 动态调整
 */
import { Comment, Fragment, Text, computed, onBeforeUnmount, ref, useSlots, watch } from 'vue'
import type { VNode } from 'vue'
import type {
  FlexBreakpoint,
  FlexProps,
  SpaceSize,
} from './types'
import { resolveSpaceSize } from '../shared/space-size'

/* ============================================================
 * Props
 * ============================================================ */

const props = withDefaults(defineProps<FlexProps>(), {
  direction: 'row',
  vertical: false,
  align: undefined,
  justify: 'start',
  size: 'md',
  gap: undefined,
  rowGap: undefined,
  colGap: undefined,
  wrap: false,
  inline: false,
  reverse: false,
  separator: false,
  flex: undefined,
  as: 'div',
  breakpoints: undefined,
})

const slots = useSlots()

/* ============================================================
 * 常量映射
 * ============================================================ */

/** SpaceSize 预设映射已抽取至 shared/space-size.ts(避免与 Container.vue 重复) */

/** align 预设别名 → CSS 关键字(规范 §2.4.4) */
const ALIGN_MAP: Record<string, string> = {
  start: 'flex-start',
  end: 'flex-end',
  center: 'center',
  baseline: 'baseline',
  stretch: 'stretch',
}

/** justify 预设别名 → CSS 关键字(规范 §2.4.4) */
const JUSTIFY_MAP: Record<string, string> = {
  start: 'flex-start',
  end: 'flex-end',
  center: 'center',
  'space-between': 'space-between',
  'space-around': 'space-around',
  'space-evenly': 'space-evenly',
}

/* ============================================================
 * 工具函数
 * ============================================================ */

/**
 * 把 SpaceSize / 数字 / 任意 CSS 字符串解析为 CSS 值(规范 §2.4.3)
 * 复用 shared/space-size.ts 中的 resolveSpaceSize。
 */
function resolveGapValue(value: SpaceSize | undefined | null): string | undefined {
  return resolveSpaceSize(value)
}

/**
 * 子 VNode 的 key 推断(规范 §2.4.1)
 *  - 优先取 child.key(string | number;symbol 不被 v-for key 接受)
 *  - 否则基于 type + 索引合成
 */
function getChildKey(node: VNode, index: number): string | number {
  const k = node.key
  if (
    k !== null &&
    k !== undefined &&
    k !== '' &&
    (typeof k === 'string' || typeof k === 'number')
  ) {
    return k
  }
  const t = node.type
  const typeName =
    typeof t === 'object' && t !== null && 'name' in t
      ? String((t as { name?: unknown }).name ?? 'node')
      : typeof t === 'string'
        ? t
        : 'node'
  return `${typeName}-${index}`
}

/* ============================================================
 * REQ-UI-004:`size` / `gap` 合并 + 非法值 warn
 *   - `gap` 是 `size` 的语义别名,两者等价;同时传入时 `gap` 优先
 *   - 非法值(undefined 之外的非 SpaceSize / 非二元组)→ console.warn
 * ============================================================ */

/**
 * 校验 size/gap 的合法性:
 *  - undefined → 合法(未传)
 *  - number → 合法
 *  - string → 合法(SpaceSize 预设 或 任意 CSS 字符串)
 *  - [SpaceSize, SpaceSize] 元组 → 合法
 *  - 其它 → 非法
 */
function isValidSizeLike(v: unknown): boolean {
  if (v === undefined || v === null) return true
  if (typeof v === 'number') return true
  if (typeof v === 'string') return true
  if (Array.isArray(v) && v.length === 2) {
    return v.every((item) => typeof item === 'number' || typeof item === 'string')
  }
  return false
}

/** 运行时合并:`gap` 优先,非法值 console.warn */
const effectiveSizeProp = computed<FlexProps['size']>(() => {
  const gap = props.gap
  const size = props.size
  if (gap !== undefined) {
    if (!isValidSizeLike(gap)) {
      if (import.meta.env && import.meta.env.DEV) {
        console.warn(
          `[IcePaw IpFlex] gap=${JSON.stringify(gap)} 非法;` +
            `应为 SpaceSize('xs'|'sm'|'md'|'lg'|'xl'|number)或 [SpaceSize, SpaceSize]。已忽略。`,
        )
      }
      return size
    }
    return gap
  }
  if (size !== undefined && !isValidSizeLike(size)) {
    if (import.meta.env && import.meta.env.DEV) {
      console.warn(
        `[IcePaw IpFlex] size=${JSON.stringify(size)} 非法;` +
          `应为 SpaceSize('xs'|'sm'|'md'|'lg'|'xl'|number)或 [SpaceSize, SpaceSize]。已忽略。`,
      )
    }
    return undefined
  }
  return size
})

/* ============================================================
 * REQ-UI-004A:breakpoints 排序与激活
 *   - 收集所有断点配置,按 width 升序排序
 *   - 容器宽度 >= 断点 width 时,该断点"生效"
 *   - 选取"宽度最大且 <= 容器宽度"的断点;若均不满足,使用基础 props
 *   - 字段级覆盖:断点中未提供的字段使用基础 props
 * ============================================================ */

/**
 * 排序后的断点列表(按 width 升序);空数组表示未启用 breakpoints
 */
const sortedBreakpoints = computed<Array<[string, FlexBreakpoint & { width: number }]>>(() => {
  const bp = props.breakpoints
  if (!bp || typeof bp !== 'object') return []
  const list: Array<[string, FlexBreakpoint & { width: number }]> = []
  for (const [key, value] of Object.entries(bp)) {
    if (!value || typeof value !== 'object') continue
    const w = typeof value.width === 'number' && Number.isFinite(value.width) ? value.width : 0
    list.push([key, { ...value, width: w }])
  }
  list.sort((a, b) => a[1].width - b[1].width)
  return list
})

/** 容器宽度(由 ResizeObserver 实时更新) */
const containerWidth = ref<number>(0)
/** 容器 ref */
const containerRef = ref<HTMLElement | null>(null)

/**
 * 当前激活的断点 key(用于渲染调试);null 表示未匹配任何断点
 */
const activeBreakpointKey = computed<string | null>(() => {
  const list = sortedBreakpoints.value
  if (list.length === 0 || containerWidth.value === 0) return null
  let key: string | null = null
  for (const [k, bp] of list) {
    if (containerWidth.value >= bp.width) key = k
  }
  return key
})

/** 当前激活断点的覆盖配置(可能为 null) */
const activeBreakpoint = computed<FlexBreakpoint | null>(() => {
  const k = activeBreakpointKey.value
  if (!k) return null
  const list = sortedBreakpoints.value
  const found = list.find(([key]) => key === k)
  return found ? found[1] : null
})

/* ============================================================
 * ResizeObserver 监听容器宽度
 * ============================================================ */
let ro: ResizeObserver | null = null

function initObserver(el: HTMLElement | null): void {
  if (ro) {
    ro.disconnect()
    ro = null
  }
  if (!el) return
  if (typeof ResizeObserver === 'undefined') return
  // 首次同步当前宽度
  containerWidth.value = el.clientWidth || el.getBoundingClientRect().width || 0
  ro = new ResizeObserver((entries) => {
    for (const entry of entries) {
      const w = entry.contentRect?.width ?? 0
      if (w > 0) containerWidth.value = w
    }
  })
  ro.observe(el)
}

onBeforeUnmount(() => {
  if (ro) {
    ro.disconnect()
    ro = null
  }
})

// 响应容器 ref 变化(模板中的 ref) 与 breakpoints 启用状态
watch(
  [containerRef, () => props.breakpoints],
  ([el]) => {
    if (!props.breakpoints || Object.keys(props.breakpoints).length === 0) {
      if (ro) {
        ro.disconnect()
        ro = null
      }
      containerWidth.value = 0
      return
    }
    initObserver(el as HTMLElement | null)
  },
  { immediate: true, flush: 'post' },
)

/* ============================================================
 * 派生样式
 * ============================================================ */

/**
 * gap 样式(规范 §2.4.3 + REQ-UI-004 + REQ-UI-004A)
 * 优先级 1:rowGap / colGap 单独覆盖
 * 优先级 2:size 数组形式 [row, col]
 * 优先级 3:size / gap 单一值
 *
 * 断点覆盖:断点中提供的 gap/rowGap/colGap 单独覆盖
 */
const gapStyle = computed<Record<string, string>>(() => {
  const styles: Record<string, string> = {}

  // 断点覆盖(优先级最高,在基础 rowGap/colGap 之上)
  const bp = activeBreakpoint.value
  if (bp) {
    if (bp.rowGap !== undefined || bp.colGap !== undefined) {
      const row = resolveGapValue(bp.rowGap)
      const col = resolveGapValue(bp.colGap)
      if (row !== undefined) styles.rowGap = row
      if (col !== undefined) styles.columnGap = col
      return styles
    }
    if (bp.gap !== undefined) {
      const single = resolveGapValue(bp.gap as SpaceSize)
      if (single !== undefined) {
        styles.gap = single
        return styles
      }
      if (Array.isArray(bp.gap)) {
        const [row, col] = bp.gap as [SpaceSize, SpaceSize]
        const r = resolveGapValue(row)
        const c = resolveGapValue(col)
        if (r !== undefined) styles.rowGap = r
        if (c !== undefined) styles.columnGap = c
        return styles
      }
    }
  }

  // 基础 rowGap / colGap 单独覆盖
  if (props.rowGap !== undefined || props.colGap !== undefined) {
    const row = resolveGapValue(props.rowGap)
    const col = resolveGapValue(props.colGap)
    if (row !== undefined) styles.rowGap = row
    if (col !== undefined) styles.columnGap = col
    return styles
  }

  // size / gap 二元组形式
  const sizeVal = effectiveSizeProp.value
  if (Array.isArray(sizeVal)) {
    const [row, col] = sizeVal as [SpaceSize, SpaceSize]
    const r = resolveGapValue(row)
    const c = resolveGapValue(col)
    if (r !== undefined) styles.rowGap = r
    if (c !== undefined) styles.columnGap = c
    return styles
  }

  // size / gap 单一值
  const single = resolveGapValue(sizeVal as SpaceSize)
  if (single !== undefined) styles.gap = single
  return styles
})

/**
 * flex 自身属性 + align/justify 样式(规范 §2.4.4 + REQ-UI-004A)
 *  - 数字 flex 简写:flex=1 → '1 1 0%'
 *  - align 接受预设别名或任意 CSS 值
 *  - 断点中提供的字段优先于基础 props
 */
const flexStyle = computed<Record<string, string>>(() => {
  const styles: Record<string, string> = {}
  const bp = activeBreakpoint.value

  if (props.flex !== undefined) {
    styles.flex = typeof props.flex === 'number'
      ? `${props.flex} ${props.flex} 0%`
      : props.flex
  }

  const align = bp?.align ?? props.align
  if (align) {
    const mapped = ALIGN_MAP[align]
    styles.alignItems = mapped ?? align
  }

  const justify = bp?.justify ?? props.justify
  if (justify) {
    const mapped = JUSTIFY_MAP[justify]
    styles.justifyContent = mapped ?? justify
  }

  return styles
})

/* ============================================================
 * direction / vertical / reverse 三者优先级(规范 §2.4.5 + REQ-UI-004A)
 *   reverse > vertical > direction
 *   断点 direction / reverse 优先
 * ============================================================ */

/** 断点级 reverse,未提供则使用基础 reverse */
const effectiveReverse = computed<boolean>(() => {
  const bp = activeBreakpoint.value
  if (bp && bp.reverse !== undefined) return bp.reverse
  return props.reverse
})

/** 断点级 direction;基础 props 中 vertical 别名优先 */
const baseDirection = computed<FlexProps['direction']>(() => {
  // 1. direction 显式设置为非默认值(非 'row')时直接返回
  if (props.direction !== 'row' || props.vertical || effectiveReverse.value) {
    if (props.direction !== 'row') return props.direction
  }
  // 2. vertical 别名
  if (props.vertical) return 'column'
  // 3. 默认
  return 'row'
})

const resolvedDirection = computed<FlexProps['direction']>(() => {
  const bp = activeBreakpoint.value
  if (bp && bp.direction !== undefined) return bp.direction
  return baseDirection.value
})

const finalDirection = computed<FlexProps['direction']>(() => {
  const base = resolvedDirection.value
  if (!effectiveReverse.value) return base
  // reverse 叠加 / 反转
  if (base === 'row') return 'row-reverse'
  if (base === 'column') return 'column-reverse'
  return base === 'row-reverse' ? 'row' : 'column'
})

/* ============================================================
 * wrap 解析(规范 §2.3 + REQ-UI-004A)
 *   boolean: true → 'wrap', false → 'nowrap'
 *   字符串: 原样使用
 *   断点 wrap 覆盖
 * ============================================================ */

const wrapValue = computed<string>(() => {
  const bp = activeBreakpoint.value
  const w = bp?.wrap !== undefined ? bp.wrap : props.wrap
  if (typeof w === 'boolean') {
    return w ? 'wrap' : 'nowrap'
  }
  return w
})

/* ============================================================
 * inline(规范 §2.3 + REQ-UI-004A)
 * ============================================================ */
const isInline = computed<boolean>(() => {
  const bp = activeBreakpoint.value
  if (bp && bp.inline !== undefined) return bp.inline
  return props.inline
})

/* ============================================================
 * 根 class / style 汇总
 * ============================================================ */

const rootClass = computed<string[]>(() => {
  const list: string[] = ['ip-flex']
  if (isInline.value) list.push('ip-flex--inline')

  // direction 修饰类
  switch (finalDirection.value) {
    case 'column':
      list.push('ip-flex--column')
      break
    case 'row-reverse':
      list.push('ip-flex--row-reverse')
      break
    case 'column-reverse':
      list.push('ip-flex--column-reverse')
      break
    default:
      list.push('ip-flex--row')
  }

  // wrap 修饰类
  if (wrapValue.value === 'wrap') list.push('ip-flex--wrap')
  else if (wrapValue.value === 'wrap-reverse') list.push('ip-flex--wrap-reverse')
  else if (wrapValue.value === 'nowrap') list.push('ip-flex--nowrap')

  // separator 模式:加 row/column 辅助类(规范 §2.4.2 CSS 选择器依赖)
  if (hasSeparator.value) {
    list.push('ip-flex--with-separator')
    if (finalDirection.value === 'column' || finalDirection.value === 'column-reverse') {
      list.push('ip-flex--column')
    } else {
      list.push('ip-flex--row')
    }
  }

  return list
})

const rootStyle = computed<Record<string, string>>(() => ({
  ...gapStyle.value,
  ...flexStyle.value,
}))

/* ============================================================
 * Separator 模式
 *   关键:过滤掉注释、空白、v-if=false 节点(规范 §2.4.1)
 * ============================================================ */

const hasSeparator = computed<boolean>(() => {
  if (typeof props.separator === 'boolean') return props.separator
  if (typeof props.separator === 'string') return props.separator.length > 0
  return !!props.separator
})

/**
 * 过滤有效子 VNode(规范 §2.4.1)
 * 排除:注释节点、空文本、Fragment 需展开
 */
function isValidVNode(node: VNode | undefined | null): boolean {
  if (!node) return false
  // 注释节点(v-if=false 也会被 Vue 编译为注释节点)
  if (node.type === Comment) return false
  // 空文本
  if (node.type === Text) {
    return typeof node.children === 'string'
      ? node.children.trim().length > 0
      : true
  }
  return true
}

/**
 * 扁平化 Fragment 子节点
 */
function flattenChildren(nodes: VNode[]): VNode[] {
  const out: VNode[] = []
  for (const node of nodes) {
    if (!node) continue
    if (node.type === Fragment && Array.isArray(node.children)) {
      out.push(...flattenChildren(node.children as VNode[]))
    } else if (isValidVNode(node)) {
      out.push(node)
    }
  }
  return out
}

const validChildren = computed<VNode[]>(() => {
  if (!hasSeparator.value) return []
  const raw = slots.default?.() ?? []
  return flattenChildren(raw as VNode[])
})

/**
 * 分隔符节点的 class
 *  - string 形式:文本分隔符
 *  - boolean 形式:线分隔符
 *  - VNode 形式:由 slot 决定(使用 ip-flex__separator--custom 容器)
 */
const separatorClass = computed<string[]>(() => {
  const list: string[] = ['ip-flex__separator']
  if (typeof props.separator === 'string') {
    list.push('ip-flex__separator--text')
  } else if (props.separator === true) {
    list.push('ip-flex__separator--line')
  } else {
    list.push('ip-flex__separator--custom')
  }
  return list
})

/* ============================================================
 * 开发模式警告:迁移提示
 *   v1.0 → v1.1:direction 从 'horizontal' / 'vertical' 改为 'row' / 'column'
 *   (规范 §2.4.5 + 验收清单最后一项)
 * ============================================================ */

if (import.meta.env && import.meta.env.DEV) {
  // 运行时类型守卫:在开发模式下捕获旧值 'horizontal' / 'vertical'(规范 §2.4.5)
  const dir = props.direction as string
  if (dir === 'horizontal' || dir === 'vertical') {
    console.warn(
      `[IcePaw IpFlex] direction="${dir}" 已废弃(规范 v1.1):` +
        `请改用 "row" 或 "column"。` +
        `v1.1 起 IpFlex 沿用 CSS 原生 direction 命名;` +
        `vertical 请改用 vertical=true 或 direction="column"。`,
    )
  }
}
</script>

<template>
  <!-- 模式 A:Gap 模式(默认,separator=false / undefined)— 零包裹(规范 §2.4.1) -->
  <component
    :is="as"
    v-if="!hasSeparator"
    ref="containerRef"
    :class="rootClass"
    :style="rootStyle"
  >
    <slot />
  </component>

  <!-- 模式 B:Separator 模式 — 遍历子元素 + 插入分隔符(规范 §2.4.1) -->
  <component
    :is="as"
    v-else
    ref="containerRef"
    :class="rootClass"
    :style="rootStyle"
  >
    <template
      v-for="(child, idx) in validChildren"
      :key="getChildKey(child, idx)"
    >
      <component
        :is="'span'"
        v-if="idx > 0"
        :class="separatorClass"
        :aria-hidden="true"
      >
        <slot name="separator" :index="idx">
          <!-- separator=true → 默认竖线/横线;string → 居中文本;VNode → 渲染传入节点(规范 §2.5.1) -->
          <template v-if="typeof props.separator === 'string'">
            {{ props.separator }}
          </template>
          <component
            :is="props.separator"
            v-else-if="props.separator && typeof props.separator === 'object'"
          />
        </slot>
      </component>
      <component :is="child" />
    </template>
  </component>
</template>

<style scoped>
/* ============================================================
 * IpFlex — 根容器(规范 §2.4.2)
 * ============================================================ */
.ip-flex {
  display: flex;            /* 默认 block 级(inline=false) */
  flex-direction: row;      /* direction 默认 */
  /* gap 由 inline style 注入(运行计算) */
}
.ip-flex--inline {
  display: inline-flex;
}

/* direction 修饰类(规范 §2.4.2) */
.ip-flex--row            { flex-direction: row; }
.ip-flex--column         { flex-direction: column; }
.ip-flex--row-reverse    { flex-direction: row-reverse; }
.ip-flex--column-reverse { flex-direction: column-reverse; }

/* wrap 修饰类(规范 §2.4.2) */
.ip-flex--wrap          { flex-wrap: wrap; }
.ip-flex--wrap-reverse  { flex-wrap: wrap-reverse; }
.ip-flex--nowrap        { flex-wrap: nowrap; }

/* ============================================================
 * Separator 模式(规范 §2.4.2 + §2.5.2)
 *   - 行内(默认):1px 竖线
 *   - 纵向(column):1px 横线
 * ============================================================ */
.ip-flex__separator {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

.ip-flex--with-separator.ip-flex--row .ip-flex__separator--line {
  width: 1px;
  align-self: stretch;
  background: var(--ip-color-border-default);
}

.ip-flex--with-separator.ip-flex--column .ip-flex__separator--line {
  height: 1px;
  width: 100%;
  background: var(--ip-color-border-default);
}

.ip-flex__separator--text {
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-caption-size);
  user-select: none;
  display: inline-flex;
  align-items: center;
  white-space: pre;
}

.ip-flex__separator--custom {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}
</style>