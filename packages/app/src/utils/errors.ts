/** 后端错误信息 → 用户友好中文提示 */
const ERROR_MAP: [RegExp, string][] = [
  [/rate.?limit/i, "请求过于频繁，请稍后再试"],
  [/timed?out/i, "请求超时，请检查网络后重试"],
  [/unauthorized|invalid.*(api.?key|key)/i, "API Key 无效，请在设置中重新配置"],
  [/insufficient_quota|quota.*exceeded|billing/i, "API 额度不足，请检查账户余额"],
  [/server.*error|internal.*server/i, "服务端暂时异常，请稍后重试"],
  [/network|dns|connect.*refused|tcp/i, "网络连接失败，请检查网络设置"],
  [/overloaded/i, "服务繁忙，请稍后重试"],
  [/context_length|token.*(limit|exceed)/i, "对话过长，请缩短上下文或开启新对话"],
  [/parse|invalid.*json/i, "响应解析失败，请重试"],
  [/cancelled|canceled|aborted/i, "请求已取消"],
];

/** 将后端原始错误信息映射为中文用户友好提示 */
export function friendlyError(raw: string | null | undefined): string {
  if (!raw) return "发生未知错误，请重试";
  for (const [re, msg] of ERROR_MAP) {
    if (re.test(raw)) return msg;
  }
  // 兜底：截断过长原始错误
  return raw.length > 80 ? raw.substring(0, 80) + "…" : raw;
}
