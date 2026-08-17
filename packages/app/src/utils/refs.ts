// @ 引用辅助：稳定短码（@会话 / @agent / @消息 的 display 消歧后缀）
//
// 用 FNV-1a 32 位哈希取 4 位数字（% 10000）：同一 id 永远同一短码，跨端不漂移。
// 不用 rowid——SQLite VACUUM 可重排物理行号。碰撞（万分之一级）由名称前缀消歧。
// 后端只认 target_id，短码纯装饰（不参与任何查找）。

/** FNV-1a 32 位 → 4 位数字短码（前导零补齐）。 */
export function shortCode(id: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return String((h >>> 0) % 10000).padStart(4, "0");
}

/** 消息 content_blocks 里的 Reference 块（ChatMessages 渲染引用卡片用）。 */
export interface ParsedRef {
  refKind: "conversation" | "agent" | "message";
  targetId: string;
  display: string;
}

/** 解析 content_blocks 中的 reference 块（同 parseAttachmentBlocks 模式）。 */
export function parseReferenceBlocks(contentBlocks: string | null | undefined): ParsedRef[] {
  if (!contentBlocks || contentBlocks === "[]") return [];
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter(
      (b): b is { ref_kind: ParsedRef["refKind"]; target_id: string; display: string } =>
        typeof b === "object" && b !== null && (b as Record<string, unknown>).type === "reference",
    ).map((b) => ({ refKind: b.ref_kind, targetId: b.target_id, display: b.display }));
  } catch {
    return [];
  }
}

/**
 * 消息 id → 所在消息组的组首 id（@消息 跳转定位用）。
 * ChatMessages 的 DOM 定位符 data-mid 挂在组首（连续同 role 合并组）；引用目标
 * 可能是组中任意一条（历史引用/乐观 id 刷新后），向前回溯到 role 变化处即组首。
 * 未命中返回原 id（调用方按 data-mid 直查，查不到自有兜底）。
 */
export function resolveGroupMid<T extends { id: string; role: string }>(
  messages: T[],
  messageId: string,
): string {
  const idx = messages.findIndex((m) => m.id === messageId);
  if (idx < 0) return messageId;
  let head = idx;
  while (head > 0 && messages[head - 1].role === messages[idx].role) head--;
  return messages[head].id;
}
