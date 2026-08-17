// taskBudget.test.ts — 任务列高度预算截断的纯函数单测（规模治理二轮，2026-08-17）。
// jsdom 无布局测不了组件层高度逻辑，语义全部收拢在此（见 taskBudget.ts 头注）。
import { describe, it, expect } from "vitest";
import { budgetDoneRows } from "../taskBudget";

describe("budgetDoneRows（非 running 任务可见行数）", () => {
  it("预算内全显：slots >= doneN → doneN", () => {
    expect(budgetDoneRows(10, 0, 10)).toBe(10);
    expect(budgetDoneRows(10, 3, 5)).toBe(5);
    expect(budgetDoneRows(12, 2, 10)).toBe(10);
  });

  it("超出预算：让一行给「还有 N 个」计数行 → slots - 1", () => {
    expect(budgetDoneRows(7, 0, 10)).toBe(6);
    expect(budgetDoneRows(7, 2, 10)).toBe(4); // running 先占 2 槽
  });

  it("恰好放满（slots == doneN）不截断；差 1 才让位", () => {
    expect(budgetDoneRows(9, 0, 10)).toBe(8);
    expect(budgetDoneRows(10, 0, 10)).toBe(10);
  });

  it("running 吃满/超出预算：非 running 全收进计数行 → 0", () => {
    expect(budgetDoneRows(3, 3, 10)).toBe(0);
    expect(budgetDoneRows(3, 5, 10)).toBe(0);
  });

  it("无非 running 任务 → 0", () => {
    expect(budgetDoneRows(5, 2, 0)).toBe(0);
  });

  it("最小预算：1 行也给计数行让位（doneN>0 时仍可读全量入口）", () => {
    expect(budgetDoneRows(1, 0, 5)).toBe(0);
  });
});
