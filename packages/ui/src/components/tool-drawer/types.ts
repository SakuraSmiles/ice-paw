/**
 * ToolDrawer — 公开 Props / Emits / 类型定义
 *
 * 规范：icepaw-tool-drawer-specs.md §二
 * 场景：ChatInput 输入区上方折叠抽屉，承载 Templates/Tools/Model 三个 Tab
 */

/** Tab 定义 */
export interface ToolDrawerTab {
  /** Tab 唯一标识（如 'templates' / 'tools' / 'model'） */
  id: string
  /** Tab 显示文案 */
  label: string
}

export interface ToolDrawerProps {
  /** 受控展开/折叠（v-model:open） */
  open: boolean

  /** 当前激活 Tab id。默认第一个 tab 的 id */
  activeTab?: string

  /** Tab 列表 */
  tabs: ToolDrawerTab[]

  /** 折叠态按钮文案。默认 '+ Tools' */
  toggleLabel?: string

  /** 展开态关闭按钮 aria-label。默认 '收起工具面板' */
  closeLabel?: string

  /** 抽屉最大高度（超出滚动）。默认 280px */
  maxHeight?: number | string

  /** 禁用（流式中自动锁定折叠） */
  disabled?: boolean

  /** 自定义根节点 aria-label */
  ariaLabel?: string
}

export interface ToolDrawerEmits {
  /** 展开/折叠切换 */
  (e: 'update:open', value: boolean): void
  /** Tab 切换 */
  (e: 'tabChange', tabId: string): void
  /** 展开动画完成 */
  (e: 'expanded'): void
  /** 折叠动画完成 */
  (e: 'collapsed'): void
}
