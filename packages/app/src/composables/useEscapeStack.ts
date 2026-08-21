// useEscapeStack — 全局 Esc 关闭栈（UI-5，2026-08-21）
//
// 问题：多个浮层（图片预览/附件详情/下拉/@弹层）各自监听 Esc，互相不感知——
// 同一次 Esc 可能关掉多层，或被下层未解绑的监听吞掉。
//
// 方案：模块级单栈（后开者在上），Esc 只触发栈顶的关闭回调；组件挂载时
// push、卸载自动 pop（onScopeDispose，无需手动清理）。注册函数返回当前
// 句柄供测试/显式退栈。守卫：事件已处理（stack 消费）则不冒泡到其他监听。
//
// 用法：
//   const { pop } = useEscapeStack(() => close());
//   // 浮层关闭时务必 pop()（或依赖组件卸载自动退栈）
import { onScopeDispose } from "vue";

type Entry = { close: () => void };

const stack: Entry[] = [];

function onGlobalKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape" || stack.length === 0) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  stack[stack.length - 1].close();
}

let installed = false;
function install() {
  if (installed) return;
  installed = true;
  window.addEventListener("keydown", onGlobalKeydown, true); // capture：先于组件自身监听
}

/** 注册一个 Esc 关闭回调（栈顶优先）。组件作用域结束自动退栈。 */
export function useEscapeStack(close: () => void): { pop: () => void } {
  install();
  const entry: Entry = { close };
  stack.push(entry);
  const pop = () => {
    const i = stack.indexOf(entry);
    if (i >= 0) stack.splice(i, 1);
  };
  onScopeDispose(pop);
  return { pop };
}
