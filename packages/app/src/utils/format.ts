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
