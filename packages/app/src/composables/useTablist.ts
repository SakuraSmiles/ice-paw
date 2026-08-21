// useTablist — WAI-ARIA tablist 键盘协议（UI-D2，2028-08-21）
//
// roving tabindex 模式：组内仅激活 tab 可 Tab 进入（其余 tabindex=-1），
// ←/→ 循环移动且焦点跟随激活（focus 即 select，ARIA 推荐），Home/End 跳边界。
// 读屏器经 role=tablist/tab + aria-selected 获知互斥视图切换语义。
//
// 用法：
//   const { onKeydown, setTab } = useTablist(tabs, modelValue)
//   <nav role="tablist" @keydown="onKeydown">
//     <button v-for="t in tabs" role="tab" :aria-selected="modelValue===t"
//            :tabindex="modelValue===t ? 0 : -1" @click="setTab(t)">…</button>


export function useTablist<T extends string>(
  tabs: readonly T[],
  active: () => T,
  setActive: (t: T) => void,
): { onKeydown: (e: globalThis.KeyboardEvent) => void } {
  const move = (dir: 1 | -1) => {
    const cur = tabs.indexOf(active());
    if (cur === -1) return;
    const next = tabs[(cur + dir + tabs.length) % tabs.length];
    setActive(next);
    // 焦点跟随激活：新 tab 按钮获得焦点（roving tabindex 的另一半）
    requestAnimationFrame(() => {
      const el = document.querySelector<HTMLElement>(
        `[role="tab"][data-tab="${next}"]`,
      );
      el?.focus();
    });
  };

  const onKeydown = (e: globalThis.KeyboardEvent) => {
    switch (e.key) {
      case "ArrowRight": e.preventDefault(); move(1); break;
      case "ArrowLeft": e.preventDefault(); move(-1); break;
      case "Home": e.preventDefault(); setActive(tabs[0]); break;
      case "End": e.preventDefault(); setActive(tabs[tabs.length - 1]); break;
    }
  };

  return { onKeydown };
}
