// useKeepAliveListeners — keep-alive 静默门控 + 监听器生命周期（U3-5 ② 事件接线脚手架收敛）。
//
// useProjectTasks / useProjectTrajectory 曾各自手抄一份「listenerLive 门控 +
// unlisteners 收集 + onDeactivated/onActivated 暂停恢复 + onBeforeUnmount 拆卸」
// 样板——这是 2026-08-31 生产卡顿修复（路由级 keep-alive 缓存下 onBeforeUnmount
// 在离开路由时不触发、监听器会挂满整个应用生命周期空转）的两个项目页同款复制。
// 收敛到本 composable：门控/收集/拆卸三件套单一真相源，防「改一处漏一处」漂移。
//
// 语义（两调用方一致，勿单边改）：
// - onDeactivated 丢弃离场期事件、onActivated 恢复；错过的增量由页面层
//   onActivated 补拉兜底（事件不重放，回页拉一次即追平）
// - 非 keep-alive 环境两钩子不触发、flag 恒 true，行为与旧版一致
import { onActivated, onBeforeUnmount, onDeactivated } from "vue";

export interface KeepAliveListeners {
  /** 当前是否 live（onDeactivated 后 false、onActivated 恢复；非 keep-alive 恒 true） */
  isLive: () => boolean;
  /** 登记一个 Tauri 监听器（`register(await listen(...))`），卸载时统一拆卸 */
  register: (unlisten: () => void) => void;
}

export function useKeepAliveListeners(): KeepAliveListeners {
  let listenerLive = true;
  const unlisteners: Array<() => void> = [];

  onDeactivated(() => { listenerLive = false; });
  onActivated(() => { listenerLive = true; });
  onBeforeUnmount(() => {
    unlisteners.forEach((u) => u());
    unlisteners.length = 0;
  });

  return {
    isLive: () => listenerLive,
    register: (unlisten) => { unlisteners.push(unlisten); },
  };
}
