// utils/markdown.ts
// Markdown 渲染前的容错预处理（纯函数，方便单测）。
//
// 覆盖两类模型常见的表格输出质量问题：
//  1) GFM 表格分隔行不合规 —— 模型把分隔行退化成空 pipe（| |）、列数少于表头、
//     用了非 ASCII 破折号（| — | — |）、或某个单元格漏了 -。markdown-it 要求分隔行
//     每列至少一个 -，否则整表退化成普通段落（用户看到一堆竖线）。
//     处理：检测「pipe 表头 + 紧跟 pipe-only 分隔候选」且该行无法为每个表头列提供 -，
//     按表头列数重建为 | --- | --- | ... |。
//  2) 表格紧跟段落/列表、中间无空行 —— markdown-it 把表头当 lazy-continuation 吞进
//     上一段（再叠加 typographer 把分隔行 --- 转成 —，渲染成竖线乱码）。
//     处理：给「紧跟在非表格行之后的表格骨架」补一个空行。
//
// 只改渲染、不改存储内容；触发条件极窄（仅动 pipe + 空白 + -/:/破折号组成的行），
// 数据行含文字永远不会被碰到，不会把非表格误变成表格。

/** 拆分 pipe 行的单元格：去掉首尾因定界 `|` 产生的空串。非 pipe 行返回 null。 */
function pipeCells(line: string): string[] | null {
  const s = line.trim();
  if (!s.includes("|")) return null;
  const parts = s.split("|");
  if (parts.length > 0 && parts[0].trim() === "") parts.shift();
  if (parts.length > 0 && parts[parts.length - 1].trim() === "") parts.pop();
  return parts;
}

/** 分隔行候选：pipe 包住、每个单元格只含 空格 / - / : / 各种 ASCII 与 Unicode 破折号。 */
function isSeparatorCandidate(line: string): boolean {
  const cells = pipeCells(line);
  if (cells === null) return false;
  // - : 为 GFM 合法分隔字符；—–─━ 为模型常误用的各种破折号（em/en/box drawing）
  return cells.every((c) => /^[\s:\-—–─━]*$/.test(c));
}

/** 重建合法分隔行：按列数 N 生成 `| --- | --- | ... |`。 */
function buildSeparator(count: number): string {
  return "| " + Array.from({ length: count }, () => "---").join(" | ") + " |";
}

/**
 * 愈合不合规的分隔行。仅当某行是 pipe 表头（至少一个非空单元格）、
 * 紧跟一行 pipe-only 分隔候选、且该分隔行无法为每个表头列提供 `-` 时，
 * 按表头列数重建该分隔行。合法分隔行（每列都有 -）原样不动。
 */
export function healTableSeparators(src: string): string {
  const lines = src.split("\n");
  for (let i = 0; i < lines.length - 1; i++) {
    const header = pipeCells(lines[i]);
    if (!header || header.length === 0) continue;
    // 表头必须至少一个非空单元格，否则空 pipe 行（| |）不会被误当表头
    if (!header.some((c) => c.trim() !== "")) continue;
    if (!isSeparatorCandidate(lines[i + 1])) continue;
    const n = header.length;
    const sep = pipeCells(lines[i + 1]) ?? [];
    const insufficient =
      sep.length < n || sep.slice(0, n).some((c) => !c.includes("-"));
    if (insufficient) {
      lines[i + 1] = buildSeparator(n);
    }
  }
  return lines.join("\n");
}

/**
 * 给「紧跟在非表格行之后的表格骨架」补一个空行，避免表头被 lazy-continuation
 * 吞进上一段。
 */
export function ensureBlankLineBeforeTables(src: string): string {
  // $1 = 普通文本行末字符 + 换行；$2 = 表头行 + 分隔行（GFM 表格最小骨架）
  return src.replace(/([^\n|]\n)(\|[^\n]*\|\s*\n\|[\s:|-]+\|)/g, "$1\n$2");
}

/** Markdown 渲染前容错预处理：先愈合分隔行，再补表前空行。 */
export function preprocessMarkdown(src: string): string {
  return ensureBlankLineBeforeTables(healTableSeparators(src));
}
