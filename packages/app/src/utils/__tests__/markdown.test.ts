import { describe, it, expect } from "vitest";
import {
  healTableSeparators,
  ensureBlankLineBeforeTables,
  preprocessMarkdown,
} from "../markdown";

// 辅助：用最小 markdown-it 判定预处理后能否被识别为表格。
// 直接断言预处理后的文本结构（不引入 markdown-it 依赖），更能定位回归点。
describe("healTableSeparators", () => {
  it("愈合空 pipe 分隔行 | |（用户实测 case）", () => {
    const src = "## 一、最终确认\n\n| 文件 | 类型 | 我的处理 |\n| |\n| **SRS** | `.docx` | ok |";
    const out = healTableSeparators(src);
    expect(out).toContain("| --- | --- | --- |");
    // 表头与数据行原样保留
    expect(out).toContain("| 文件 | 类型 | 我的处理 |");
    expect(out).toContain("| **SRS** | `.docx` | ok |");
  });

  it("愈合列数不足的分隔行（表头3列，分隔2列）", () => {
    const src = "| 文件 | 类型 | 我的处理 |\n| --- | --- |\n| a | b | c |";
    expect(healTableSeparators(src)).toContain("| --- | --- | --- |");
  });

  it("愈合 em-dash 分隔行（模型直接吐非ASCII破折号）", () => {
    const src = "| 文件 | 类型 | 我的处理 |\n| — | — | — |\n| a | b | c |";
    expect(healTableSeparators(src)).toContain("| --- | --- | --- |");
  });

  it("愈合中间单元格漏 - 的分隔行", () => {
    const src = "| 文件 | 类型 | 我的处理 |\n| --- | | --- |\n| a | b | c |";
    expect(healTableSeparators(src)).toContain("| --- | --- | --- |");
  });

  it("合法分隔行（每列都有 -）原样不动", () => {
    const src = "| 文件 | 类型 | 我的处理 |\n| --- | --- | --- |\n| a | b | c |";
    expect(healTableSeparators(src)).toBe(src);
  });

  it("带冒号对齐的合法分隔行原样不动", () => {
    const src = "| 文件 | 类型 |\n| :--- | ---: |\n| a | b |";
    expect(healTableSeparators(src)).toBe(src);
  });

  it("不把空 pipe 段落误判成表格（表头需至少一个非空单元格）", () => {
    const src = "普通段落\n| |\n| |";
    expect(healTableSeparators(src)).toBe(src);
  });

  it("数据行不会被误当分隔行修改（含文字）", () => {
    const src = "| 文件 | 类型 |\n| --- | --- |\n| a | b |";
    expect(healTableSeparators(src)).toBe(src);
  });

  it("单列表格也能愈合", () => {
    const src = "| 标题 |\n| |\n| a |";
    expect(healTableSeparators(src)).toContain("| --- |");
  });

  it("用户完整实例（多行数据）愈合", () => {
    const src =
      "## 一、最终确认\n\n| 文件 | 类型 | 我的处理 |\n| |\n| **SRS 最终版** | `.docx` | 直接读 |\n| 客户样例 | `.doc` | 需协助 |";
    const out = healTableSeparators(src);
    expect(out).toContain("| --- | --- | --- |");
    expect(out).toContain("| 客户样例 | `.doc` | 需协助 |");
  });
});

describe("ensureBlankLineBeforeTables", () => {
  it("给紧跟段落无空行的表格补空行", () => {
    const src = "一些文字\n| 文件 | 类型 |\n| --- | --- |\n| a | b |";
    const out = ensureBlankLineBeforeTables(src);
    expect(out).toContain("一些文字\n\n| 文件");
  });

  it("已有空行的表格不变", () => {
    const src = "一些文字\n\n| 文件 | 类型 |\n| --- | --- |\n| a | b |";
    expect(ensureBlankLineBeforeTables(src)).toBe(src);
  });
});

describe("preprocessMarkdown（组合）", () => {
  it("同时愈合分隔行 + 保留表前空行", () => {
    const src =
      "## 一、最终确认\n\n| 文件 | 类型 | 我的处理 |\n| |\n| **SRS** | `.docx` | ok |";
    const out = preprocessMarkdown(src);
    expect(out).toContain("| --- | --- | --- |");
  });

  it("坏分隔行 + 紧跟段落无空行，两路都修", () => {
    const src = "前言文字\n| a | b |\n| |\n| 1 | 2 |";
    const out = preprocessMarkdown(src);
    expect(out).toContain("前言文字\n\n| a |");
    expect(out).toContain("| --- | --- |");
  });
});
