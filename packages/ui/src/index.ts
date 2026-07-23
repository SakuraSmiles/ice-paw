/**
 * @ice-paw/ui 主入口
 *
 * 阶段 2：导出 P0 原子组件 + composables
 *  - Button / Input / Textarea / MessageBubble / Modal / Toast
 *  - IpFlex / IpContainer(布局组件，规范 icepaw-layout-system.md v1.1)
 *  - IpAvatar / IpCard / IpSelect / IpEmptyState / IpDropdownMenu / IpPopconfirm
 *    （P0 组件，规范 icepaw-p0-component-specs.md）
 *  - useToast / provideToast
 *  - 类型定义
 */

import './styles/index.css'

/* 组件 */
export { Button } from './components/button'
export type {
  ButtonProps,
  ButtonEmits,
  ButtonVariant,
  ButtonSize,
} from './components/button/types'

export { Input } from './components/input'
export type {
  InputProps,
  InputEmits,
  InputSize,
  InputState,
} from './components/input/types'

export { Textarea } from './components/textarea'
export type {
  TextareaProps,
  TextareaEmits,
  TextareaSize,
} from './components/textarea/types'

export { MessageBubble } from './components/message-bubble'
export type {
  MessageBubbleProps,
  MessageBubbleEmits,
  MessageRole,
} from './components/message-bubble/types'

export { Modal } from './components/modal'
export type {
  ModalProps,
  ModalEmits,
  ModalSize,
} from './components/modal/types'

export {
  Toast,
  ToastContainer,
  useToast,
  provideToast,
  createToastApi,
  ToastApiKey,
} from './components/toast'
export type {
  ToastType,
  ToastPosition,
  ToastOptions,
  ToastInstance,
  ToastApi,
} from './components/toast/types'

/* P0 组件（规范 icepaw-p0-component-specs.md） */
export { Avatar as IpAvatar } from './components/avatar'
export type {
  AvatarProps as IpAvatarProps,
  AvatarEmits as IpAvatarEmits,
  AvatarSource as IpAvatarSource,
  AvatarShape as IpAvatarShape,
  AvatarSize as IpAvatarSize,
  AvatarUploadError as IpAvatarUploadError,
} from './components/avatar/types'

export { Card as IpCard } from './components/card'
export type {
  CardProps as IpCardProps,
  CardEmits as IpCardEmits,
  CardVariant as IpCardVariant,
  CardPadding as IpCardPadding,
  CardAs as IpCardAs,
} from './components/card/types'

export { Select as IpSelect } from './components/select'
export type {
  SelectProps as IpSelectProps,
  SelectEmits as IpSelectEmits,
  SelectOption as IpSelectOption,
  SelectSize as IpSelectSize,
} from './components/select/types'

export { EmptyState as IpEmptyState } from './components/empty-state'
export type {
  EmptyStateProps as IpEmptyStateProps,
  EmptyStateEmits as IpEmptyStateEmits,
  EmptyStateAction as IpEmptyStateAction,
  EmptyStateIconSize as IpEmptyStateIconSize,
} from './components/empty-state/types'

export { DropdownMenu as IpDropdownMenu } from './components/dropdown'
export type {
  DropdownProps as IpDropdownMenuProps,
  DropdownEmits as IpDropdownMenuEmits,
  DropdownItem as IpDropdownMenuItem,
  DropdownPlacement as IpDropdownMenuPlacement,
} from './components/dropdown/types'

export { Popconfirm as IpPopconfirm } from './components/popconfirm'
export type {
  PopconfirmProps as IpPopconfirmProps,
  PopconfirmEmits as IpPopconfirmEmits,
  PopconfirmPlacement as IpPopconfirmPlacement,
  PopconfirmTrigger as IpPopconfirmTrigger,
} from './components/popconfirm/types'

/* 布局组件（规范 icepaw-layout-system.md v1.1 P0） */
export { Flex as IpFlex } from './components/flex'
export type {
  FlexProps as IpFlexProps,
  FlexDirection,
  FlexAlign,
  FlexJustify,
  FlexWrap,
  FlexSeparator,
  SpaceSize,
  SizeProp,
} from './components/flex/types'

export { Container as IpContainer } from './components/container'
export type {
  ContainerProps as IpContainerProps,
  ContainerMaxWidth,
  ContainerPadding,
} from './components/container/types'

/* IconPicker / AvatarUpload */
export { IconPicker as IpIconPicker } from './components/icon-picker'
export type {
  IconPickerProps as IpIconPickerProps,
  IconPickerEmits as IpIconPickerEmits,
  IconPickerCategory as IpIconPickerCategory,
} from './components/icon-picker/types'

export { AvatarUpload as IpAvatarUpload } from './components/avatar-upload'
export type {
  AvatarUploadProps as IpAvatarUploadProps,
  AvatarUploadEmits as IpAvatarUploadEmits,
  AvatarUploadErrorInfo as IpAvatarUploadErrorInfo,
} from './components/avatar-upload/types'

/* 工具 */
export {
  sizeOf,
  isEmpty,
  generateId,
  useStyleVars,
} from './utils'

/* 工具类型 */
export type { BuildPropsToProps } from './utils'

/* composables */
export { useToast as useToastComposable } from './components/toast/useToast'
