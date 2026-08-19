// blockMemo — 以输入字符串为 key 的解析结果 memo（聊天渲染热路径专用）
//
// 背景：ChatMessages 模板对每条消息调用 parse*Blocks（content_blocks JSON.parse），
// 流式期间末条消息每次更新都会触发全列表重渲染——无缓存时 = 每渲染 × 每消息
// 重新 JSON.parse，长会话下打满前端 JS 主线程。
//
// 约束（调用方须知）：
// - 被包函数必须**纯**：同输入字符串必产相同结果（content_blocks 字符串相同
//   ⇒ 内容相同 ⇒ 解析结果必相同，天然幂等，无冲突风险）。
// - 返回**同一引用**，调用方**不得 mutate**（v-for / filter / find 等只读用法）；
//   Vue 对同引用跳过无关 diff，共享引用反而是收益。
// - 超过 maxEntries 时全清重建（不做 LRU：解析便宜，全清实现少一个量级复杂度，
//   且防长会话 + 分页加载的 key 无限增长）。
// - 流式路径天然安全：freeze 前 content_blocks 恒为 "[]"（恒定 key），冻结时
//   一次性写入新 key，逐 chunk 零缓存增长。

/** 包装一个纯字符串解析函数，返回带模块级缓存的同签名函数。 */
export function memoized<T>(fn: (s: string) => T, maxEntries = 500): (s: string) => T {
  const cache = new Map<string, T>();
  return (s: string): T => {
    if (cache.has(s)) {
      return cache.get(s)!;
    }
    const v = fn(s);
    if (cache.size >= maxEntries) {
      cache.clear();
    }
    cache.set(s, v);
    return v;
  };
}
