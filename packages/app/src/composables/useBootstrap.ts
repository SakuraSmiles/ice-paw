// useBootstrap.ts — REQ-XC-007 启动状态共享 composable
//
// 职责：
//   - 提供一个跨组件共享的「启动是否完成」状态，让 AppBootstrap（触发初始化）
//     与 AppLayout（onMounted 兜底初始化）能协调不重复触发网络请求
//
// 工作原理：
//   - `markBootstrapped()`：AppBootstrap 在 await 完所有 store 后调用，标记完成
//   - `hasBootstrapped()`：AppLayout 在 onMounted 中检查，若已完成则跳过初始化
//   - 使用模块级单例 ref，跨组件共享
//
// 为什么不用 provide/inject：
//   - AppLayout 是 AppBootstrap 的子组件，但 AppBootstrap 是 async setup，
//     provide/inject 行为一致。模块级 ref 更简单且无 prop drilling。

import { ref, type Ref } from "vue";

/** 模块级单例：是否已完成启动初始化 */
const bootstrapped: Ref<boolean> = ref(false);

export function useBootstrap(): {
  hasBootstrapped: () => boolean;
  markBootstrapped: () => void;
  resetBootstrap: () => void;
} {
  return {
    /** AppLayout 在 onMounted 中检查：若已 bootstrap，则跳过自己的初始化 */
    hasBootstrapped: () => bootstrapped.value,
    /** AppBootstrap 在 await 完所有 store 后调用 */
    markBootstrapped: () => {
      bootstrapped.value = true;
    },
    /** 仅测试用：重置 bootstrap 标记 */
    resetBootstrap: () => {
      bootstrapped.value = false;
    },
  };
}