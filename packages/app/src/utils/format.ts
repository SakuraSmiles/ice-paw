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
