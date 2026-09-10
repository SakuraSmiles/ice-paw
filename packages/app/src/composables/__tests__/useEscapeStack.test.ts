// useEscapeStack.test.ts — 全局 Esc 栈三形态：栈顶优先 / 活跃谓词让路 / 全不活跃放行。
// 模块级单栈 per 测试文件隔离（vitest 模块隔离），条目用 effectScope.stop() 成对退栈。
import { describe, it, expect, vi } from "vitest";
import { effectScope } from "vue";
import { useEscapeStack } from "../useEscapeStack";

function pressEscape() {
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
}

/** bubble 探针：栈消费了事件（stopImmediatePropagation 于 window capture 期）则不达 */
function probe(): { received: () => boolean } {
  let received = false;
  window.addEventListener("keydown", () => { received = true; });
  return { received: () => received };
}

describe("useEscapeStack", () => {
  it("无谓词条目恒活跃：Esc 只关栈顶", () => {
    const scope = effectScope();
    const closeBottom = vi.fn();
    const closeTop = vi.fn();
    scope.run(() => {
      useEscapeStack(closeBottom);
      useEscapeStack(closeTop);
    });
    const p = probe();
    pressEscape();
    expect(closeTop).toHaveBeenCalledTimes(1);
    expect(closeBottom).not.toHaveBeenCalled();
    expect(p.received()).toBe(false); // 已消费：不冒泡
    scope.stop();
  });

  it("活跃谓词让路：栈顶不活跃时命中其下第一个活跃条目", () => {
    const scope = effectScope();
    let topOpen = false;
    const closeBottom = vi.fn();
    const closeTop = vi.fn();
    scope.run(() => {
      useEscapeStack(closeBottom);
      useEscapeStack(closeTop, () => topOpen);
    });
    pressEscape();
    expect(closeBottom).toHaveBeenCalledTimes(1); // 栈顶闭着 → 越过它关下层
    expect(closeTop).not.toHaveBeenCalled();
    topOpen = true;
    pressEscape();
    expect(closeTop).toHaveBeenCalledTimes(1); // 开着 → 栈顶命中，下层不动
    expect(closeBottom).toHaveBeenCalledTimes(1);
    scope.stop();
  });

  it("全不活跃 → 不消费事件（放行给组件自身 Esc 处理）", () => {
    const scope = effectScope();
    const close = vi.fn();
    scope.run(() => {
      useEscapeStack(close, () => false);
    });
    const p = probe();
    pressEscape();
    expect(close).not.toHaveBeenCalled();
    expect(p.received()).toBe(true); // 未吞事件
    scope.stop();
  });

  it("pop 手动退栈 / scope 结束自动退栈", () => {
    const scope = effectScope();
    const closeA = vi.fn();
    const closeB = vi.fn();
    let popB: () => void = () => {};
    scope.run(() => {
      useEscapeStack(closeA);
      popB = useEscapeStack(closeB).pop;
    });
    popB();
    pressEscape();
    expect(closeA).toHaveBeenCalledTimes(1); // B 已退栈，A 成栈顶
    expect(closeB).not.toHaveBeenCalled();
    closeA.mockClear();
    scope.stop(); // onScopeDispose 自动清 A
    pressEscape();
    expect(closeA).not.toHaveBeenCalled();
  });
});
