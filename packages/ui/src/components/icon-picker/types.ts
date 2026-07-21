/**
 * IconPicker — 公开 Props / Emits / 类型定义
 *
 * 规范：icepaw-p0-component-specs.md 通用约定
 * 场景：从 Lucide 图标集中搜索 / 分类浏览选择图标
 */

export interface IconPickerCategory {
  /** 分类标识 */
  id: string
  /** 分类名称 */
  label: string
  /** 该分类下的图标名称列表 */
  icons: string[]
}

export interface IconPickerProps {
  /** v-model 绑定的图标名称 */
  modelValue?: string | null

  /** 分类列表。为空时展示全部图标 */
  categories?: IconPickerCategory[]

  /** 搜索框 placeholder。默认 '搜索图标...' */
  searchPlaceholder?: string

  /** 禁用 */
  disabled?: boolean
}

export interface IconPickerEmits {
  (e: 'update:modelValue', value: string | null): void
}
