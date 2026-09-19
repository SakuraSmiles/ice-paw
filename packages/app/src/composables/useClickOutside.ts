// useClickOutside — 统一「点击浮层外即收起」监听（W2，2026-09-19）。
//
// 收编 8 处手写 document 监听：此前各组件自装自拆，形态漂移（click vs
// mousedown / 冒泡 vs capture / 常驻 handler 内查状态 vs watch 开关挂摘），
// 卸载清理靠各自成对的 onUnmounted——少写一行就泄漏全局监听。本 composable
// 统一三件事：
//   1. contains 判定（target 之外的指针事件才触发 handler）；
//   2. 按需挂摘（传 active 谓词 = 只在浮层展开期间占用 document 监听，
//      关闭即摘——常驻监听只省不亏）；
//   3. 作用域结束自动拆除（onScopeDispose，卸载零清理样板）。
//
// 与 useEscapeStack 分工：本件只管指针外点；键盘 Esc 归全局栈（显式关闭
// 语义 + 跨浮层只关栈顶，勿并入）。
//
// 用法：
//   // 浮层常驻挂载、展开才监听（ChatHeader / TaskPanel / GroupedSelect 形态）：
//   useClickOutside(zoneRef, () => { open.value = false }, () => open.value);
//   // 容器处在带 @click.stop 的父层内时须 capture（MoreMenu 形态）：
//   useClickOutside(wrapRef, close, () => open.value, { capture: true });

import { onScopeDispose, watch, type Ref } from "vue";

export interface ClickOutsideOptions {
  /** 监听事件（默认 click；输入框类浮层用 mousedown——失焦/点击生效前先收） */
  event?: "click" | "mousedown";
  /** 捕获阶段监听（默认 false）。浮层处在带 @click.stop 的容器内时须 true：
   *  冒泡被拦收不到外部点击，capture 在 stop 之前触发不受影响 */
  capture?: boolean;
}

/** 低阶：无条件绑定外点监听，返回解绑函数（生命周期自理，组件内请用包装版）。 */
export function bindClickOutside(
  target: Ref<HTMLElement | null | undefined>,
  handler: () => void,
  options: ClickOutsideOptions = {},
): () => void {
  const { event = "click", capture = false } = options;
  const listener = (e: Event) => {
    const el = target.value;
    if (el && !el.contains(e.target as Node)) handler();
  };
  document.addEventListener(event, listener, capture);
  return () => document.removeEventListener(event, listener, capture);
}

/** 组件作用域包装：传 active 谓词时随其真假挂/摘监听（浮层关着不占全局
 *  监听）；不传 = 常驻绑定。作用域结束自动拆除（含 watch 停止）。 */
export function useClickOutside(
  target: Ref<HTMLElement | null | undefined>,
  handler: () => void,
  active?: () => boolean,
  options?: ClickOutsideOptions,
): { stop: () => void } {
  if (active) {
    let stopBind: (() => void) | null = null;
    const unwatch = watch(
      active,
      (on) => {
        if (on && !stopBind) stopBind = bindClickOutside(target, handler, options);
        else if (!on && stopBind) {
          stopBind();
          stopBind = null;
        }
      },
      { immediate: true },
    );
    const stop = () => {
      unwatch();
      if (stopBind) {
        stopBind();
        stopBind = null;
      }
    };
    onScopeDispose(stop);
    return { stop };
  }
  const stop = bindClickOutside(target, handler, options);
  onScopeDispose(stop);
  return { stop };
}
