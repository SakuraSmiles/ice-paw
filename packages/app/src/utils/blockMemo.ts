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
// - 超过 maxEntries 时**清至一半**（保留近期插入的一半——Map 迭代序=插入序，
//   分代近似 LRU；键是行 content_blocks 字符串引用，messages 持有同实例，
//   未变行命中走引用相等快速路径，O(1)）。
//   ⚠️ 2026-10-26 勿回退「超限即全清」（旧实现）：500 上限 + 全清 = 大会话
//   （>500 行）下缓存等效容量 1——每次重建全量重 parse，parseResultRow 的
//   memo 在生产大会话上从未真正生效（blockParsers.perf.test 实锤：重建
//   336ms≈首建 302ms）。分代淘汰后重建 <20ms。
// - 流式路径天然安全：freeze 前 content_blocks 恒为 "[]"（恒定 key），冻结时
//   一次性写入新 key，逐 chunk 零缓存增长。
// - 内存界：值含 parse 产物（图 base64 为行字符串的拷贝）——上限 4000 行
//   覆盖单会话典型规模，跨会话累积到上限时旧会话条目自然被分代老化。

/** 包装一个纯字符串解析函数，返回带模块级缓存的同签名函数。 */
export function memoized<T>(fn: (s: string) => T, maxEntries = 4000): (s: string) => T {
  const cache = new Map<string, T>();
  return (s: string): T => {
    if (cache.has(s)) {
      return cache.get(s)!;
    }
    const v = fn(s);
    if (cache.size >= maxEntries) {
      // 清至一半（删最早插入的一半——近期会话/行保留，跨会话旧条目老化）
      const drop = cache.size - Math.floor(maxEntries / 2);
      let n = 0;
      for (const k of cache.keys()) {
        if (n >= drop) break;
        cache.delete(k);
        n += 1;
      }
    }
    cache.set(s, v);
    return v;
  };
}

