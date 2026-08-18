/**
 * 通用数字/文本格式化工具（独立纯函数，可单测）
 */

/** JSON 美化：尝试 parse + stringify 缩进，失败则原样返回 */
export function formatJson(str: string): string {
  try {
    return JSON.stringify(JSON.parse(str), null, 2);
  } catch {
    return str;
  }
}

/** JSON 截断：超过 maxLen 则截断加省略号 */
export function truncateJson(str: string, maxLen = 80): string {
  if (str.length <= maxLen) return str;
  return str.substring(0, maxLen) + "…";
}


/**
 * 格式化 token 数为人类可读短格式。
 *
 * 三档：≥10000 → 「80万」（中文万进制，预算/窗口量级的主场）；
 * ≥1000 → 「1.5K」（千位保留一位小数，仅 1000~9999 区间）；
 * 其余原数输出。0 与负数原样返回。
 */
export function formatTokenCount(n: number): string {
  if (!Number.isFinite(n) || n < 1000) return String(n);
  if (n < 10_000) {
    const k = n / 1000;
    // 1.5K / 9.9K；整数倍显示 1K / 9K
    return `${Number.isInteger(k) ? k : k.toFixed(1)}K`;
  }
  const wan = n / 10_000;
  // 80万 / 12.5万；整数倍不带小数
  return `${Number.isInteger(wan) ? wan : wan.toFixed(1)}万`;
}

/**
 * token 数的国际惯用短格式（K/M）——图表/统计卡语境。
 *
 * 与 formatTokenCount（中文万进制）分工：万进制是预算/窗口量级的主场
 * （BudgetPill 等，中文用户对「80万窗口」的直觉）；图表数字列宽敏感且
 * 面向仪表盘惯例，用 K/M。≥1M → 1.2M；≥1K → 9.8K（整数倍 9K）；
 * 其余原数。
 */
export function formatTokenCompact(n: number): string {
  if (!Number.isFinite(n) || n < 1000) return String(n);
  if (n < 1_000_000) {
    const k = n / 1000;
    return `${Number.isInteger(k) ? k : k.toFixed(1)}K`;
  }
  const m = n / 1_000_000;
  // 1.2M / 23.5M
  return `${Number.isInteger(m) ? m : m.toFixed(1)}M`;
}
