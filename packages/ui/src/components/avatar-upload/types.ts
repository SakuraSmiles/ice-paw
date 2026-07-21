/**
 * AvatarUpload — 公开 Props / Emits / 类型定义
 *
 * 规范：icepaw-p0-component-specs.md 通用约定
 * 场景：点击触发文件选择 → 弹出裁剪 Dialog → 确认后输出 base64
 * 裁剪方案：cropperjs@2（框架无关，30KB gzip）
 */

/**
 * AvatarUploadError 重命名为 AvatarUploadErrorInfo 以避免与
 * 现有 IpAvatar 组件导出的 IpAvatarUploadError 冲突。
 */
export interface AvatarUploadErrorInfo {
  /** 错误码 */
  code: 'file_too_large' | 'invalid_mime' | 'no_file' | 'load_failed'
  /** 错误信息（已本地化为中文） */
  message: string
}

/** @deprecated 使用 AvatarUploadErrorInfo 替代 */
export type AvatarUploadError = AvatarUploadErrorInfo

export interface AvatarUploadProps {
  /** v-model 绑定的图片值（URL 字符串或 base64 data URL） */
  modelValue?: string | null

  /** 最大文件字节数。默认 2MB */
  maxSize?: number

  /** 预览/裁剪后图片的圆角。默认 'circle' */
  borderRadius?: 'circle' | 'rounded'

  /** 禁用 */
  disabled?: boolean
}

export interface AvatarUploadEmits {
  /** 更新图片值（base64 data URL） */
  (e: 'update:modelValue', value: string | null): void

  /** 上传错误 */
  (e: 'upload-error', error: AvatarUploadErrorInfo): void
}
