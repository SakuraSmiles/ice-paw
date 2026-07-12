import Toast from './Toast.vue'
import ToastContainer from './ToastContainer.vue'
import { useToast, provideToast, createToastApi } from './useToast'
import { ToastApiKey } from './types'
import type {
  ToastType,
  ToastPosition,
  ToastOptions,
  ToastInstance,
  ToastApi,
} from './types'

export {
  Toast,
  ToastContainer,
  useToast,
  provideToast,
  createToastApi,
  ToastApiKey,
}

export type {
  ToastType,
  ToastPosition,
  ToastOptions,
  ToastInstance,
  ToastApi,
}

export default Toast
