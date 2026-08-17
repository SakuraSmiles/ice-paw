// taskBudget.ts — 任务列「高度预算截断」的纯函数核心（规模治理，用户拍板 2026-08-17）
//
// 心智：平铺优先——高度预算内全量展开，放不下才截断并让出一行给「还有 N 个」
// 计数行；running 恒优先占位（状态监控的主体，永不挤掉）。
// 抽纯函数：jsdom 无布局（clientHeight/offsetHeight 恒 0），组件层测不了
// 高度逻辑，逻辑全收拢在此单测覆盖；组件只负责测量与接线。

/**
 * 计算非 running 任务可见行数。
 *
 * @param rowBudget 列身可见总行数（⌊列身高 / 行高⌋，至少 1）
 * @param runningN running 任务数（优先占位，不被截断挤掉）
 * @param doneN 非 running 任务总数
 * @returns 非 running 应显示的行数：预算内全显；超出时少显示一行（让位给计数行）
 */
export function budgetDoneRows(rowBudget: number, runningN: number, doneN: number): number {
  if (doneN <= 0) return 0;
  const slots = rowBudget - runningN;
  if (slots <= 0) return 0; // running 已吃满预算：非 running 全收进计数行
  return slots >= doneN ? doneN : Math.max(0, slots - 1);
}
