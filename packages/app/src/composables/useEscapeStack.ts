// useEscapeStack — 全局 Esc 关闭栈（UI-5，2026-08-21）
//
// 问题：多个浮层（图片预览/附件详情/下拉/@弹层）各自监听 Esc，互相不感知——
// 同一次 Esc 可能关掉多层，或被下层未解绑的监听吞掉。
//
// 方案：模块级单栈（后开者在上），Esc 只触发栈顶的关闭回调；组件挂载时
// push、卸载自动 pop（onScopeDispose，无需手动清理）。注册函数返回当前
// 句柄供测试/显式退栈。守卫：事件已处理（stack 消费）则不冒泡到其他监听。
//
// 活跃谓词（2026-09-10 审计批 A1）：注册方若是「常驻挂载 + 条件浮层」形态
//（如 ChatHeader 的收件箱 popover / 删除确认条——setup 期注册、浮层按需开关），
// 必须传 active 谓词；Esc 从栈顶向下找第一个活跃条目触发，不活跃的让路。
// 不带谓词 = 恒活跃（挂载/卸载形态零改动，向后兼容）。栈内全部不活跃时
// 不消费事件——放行给组件自身的 Esc 处理（如标题编辑输入框）。
//
// 用法：
//   const { pop } = useEscapeStack(() => close());
//   const { pop } = useEscapeStack(() => close(), () => open.value);
//   // 浮层关闭时务必 pop()（或依赖组件卸载自动退栈）
import { onScopeDispose } from "vue";

type Entry = { close: () => void; active?: () => boolean };

const stack: Entry[] = [];

function onGlobalKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape" || stack.length === 0) return;
  for (let i = stack.length - 1; i >= 0; i--) {
    const entry = stack[i];
    if (entry.active && !entry.active()) continue;
    e.preventDefault();
    e.stopImmediatePropagation();
    entry.close();
    return;
  }
}

let installed = false;
function install() {
  if (installed) return;
  installed = true;
  window.addEventListener("keydown", onGlobalKeydown, true); // capture：先于组件自身监听
}

/** 注册一个 Esc 关闭回调（栈顶优先，跳过不活跃条目）。
 *  active：常驻注册的条件浮层传开关谓词；缺省 = 恒活跃。组件作用域结束自动退栈。 */
export function useEscapeStack(close: () => void, active?: () => boolean): { pop: () => void } {
  install();
  const entry: Entry = active ? { close, active } : { close };
  stack.push(entry);
  const pop = () => {
    const i = stack.indexOf(entry);
    if (i >= 0) stack.splice(i, 1);
  };
  onScopeDispose(pop);
  return { pop };
}
