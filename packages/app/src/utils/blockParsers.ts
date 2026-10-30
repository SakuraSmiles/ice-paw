// blockParsers — 消息块行级解析器（2026-10-26 从 ChatMessages.vue 抽出模块化）
//
// 抽出动机：可测性——0.9.7 生产实案（toolResultIndex 丢 memo → 生成中全量重
// parse 大会话 → 前端主线程饿死：设置二级菜单跳不动/一直加载中）在组件内
// 定义时无法被性能回归覆盖。抽到 utils 后配 blockParsers.perf.test 基准锁
// （变更单行后索引重建 < 200ms——memo 生效 ≈ms 级 vs 丢失 ≈秒级，数量级区分）。
//
// ⚠️ 勿退回 computed 内裸 JSON.parse——所有行级解析必须走 memoized
// （同串零重 parse；JS 引擎字符串 hash 有缓存，键查询 O(1)）。

import { memoized } from "./blockMemo";

/** tool_result 查询结果的图片集（view_image/show_image/UE5 提取图的渲染数据源；
 *  base64 data + mediaType，与 ImagePreview/openImagePreview 的 images 形状一致）。 */
export interface ToolResultEntry {
  content: string;
  isError: boolean;
  images: { data: string; mediaType: string }[];
}

/** 行内是否含 tool_result 块（= 工具回执数据行）。用户媒体判定用它排除：
 *  工具行的 Image 块归工具归属（→ assistant 侧图片卡），误当用户媒体会把
 *  agent 汇报图渲染成右侧「用户发出」气泡（2026-10-29 他机生产实案）。 */
export const rowHasToolResult = memoized((contentBlocks: string): boolean => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    return Array.isArray(blocks) && blocks.some(
      (b) => typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'tool_result',
    );
  } catch {
    return false;
  }
});

/** 行级 tool_result 解析（memo 按行 contentBlocks 字符串缓存——同串零重 parse）。
 *  行内顺序扫描原始 blocks：tool_result 与其后的 Image 块（工具返图：
 *  view_image/show_image/UE5 提取）按序归属到最近的 tool_result，图随条目携带。 */
export const parseResultRow = memoized(
  (contentBlocks: string): [string, ToolResultEntry][] => {
    let blocks: unknown[];
    try {
      blocks = JSON.parse(contentBlocks);
    } catch {
      return [];
    }
    if (!Array.isArray(blocks)) return [];
    const out: [string, ToolResultEntry][] = [];
    let current: ToolResultEntry | null = null;
    for (const b of blocks) {
      if (typeof b !== "object" || b === null) continue;
      const rec = b as Record<string, unknown>;
      if (rec.type === "tool_result" && typeof rec.tool_use_id === "string") {
        current = {
          content: typeof rec.content === "string" ? rec.content : "",
          isError: rec.is_error === true,
          images: [],
        };
        out.push([rec.tool_use_id, current]);
      } else if (rec.type === "image" && current) {
        // 图片外置行已由 repo 读侧水合回内联 base64
        if (typeof rec.data === "string" && rec.data) {
          current.images.push({ data: rec.data, mediaType: String(rec.media_type ?? "image/png") });
        }
      }
    }
    return out;
  },
);

/** tool_use_id → tool_result 全局索引重建（ChatMessages 的 toolResultIndex
 *  computed 体——抽到这里供性能基准与组件共用）。O(n) 合并行级 Map，未变行
 *  经 memo 零成本。 */
export function buildToolResultIndex(
  messages: { role: string; content_blocks?: string }[],
): Map<string, ToolResultEntry> {
  const idx = new Map<string, ToolResultEntry>();
  for (const m of messages) {
    if (m.role !== "user") continue;
    for (const [id, entry] of parseResultRow(m.content_blocks ?? "[]")) {
      idx.set(id, entry);
    }
  }
  return idx;
}

// —— 以下各块类型解析器自 ChatMessages.vue 抽出（2026-10-30 大文件拆分）。
// 契约不变：contentBlocks 字符串进、typed 数组出；memo 化防热路径重 parse。

/** 解析 image 块。memo 化：模板热路径每渲染每消息调用。 */
export const parseImageBlocks = memoized((contentBlocks: string): { data: string; mediaType: string }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { data: string; media_type: string; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'image'
    ).map((b) => ({ data: b.data, mediaType: b.media_type }));
  } catch { return []; }
});

/** 解析 attachment 块（附件元信息卡片：文件名 / 类型 / 字节数）。memo 化同上。 */
export const parseAttachmentBlocks = memoized((contentBlocks: string): { name: string; kind: string; size: number }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { name: string; kind: string; size: number; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'attachment'
    ).map((b) => ({ name: b.name, kind: b.kind, size: b.size }));
  } catch { return []; }
});

/** 解析 tool_use 块。memo 化：模板热路径每渲染每消息调用。 */
export const parseToolUseBlocks = memoized((contentBlocks: string): { id: string; name: string; input: string }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { id: string; name: string; input: string; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'tool_use'
    ).map((b) => ({ id: b.id, name: b.name, input: b.input }));
  } catch { return []; }
});

/** 解析 thinking 块。memo 化同上。durationMs = 后端落库的思考段耗时
 *  （旧消息无此字段 → null，label 只显示「思考」）。 */
export const parseThinkingBlocks = memoized((contentBlocks: string): { thinking: string; durationMs: number | null }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { thinking: string; duration_ms?: number; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'thinking'
    ).map((b) => ({ thinking: b.thinking, durationMs: typeof b.duration_ms === 'number' ? b.duration_ms : null }));
  } catch { return []; }
});

/** 用户消息是否含图片或附件块（纯附件无文本时仍需显示气泡）。
 *  ⚠️ tool_result 数据行不算用户媒体（2026-10-29 他机实案）：0.9.9 起工具回执
 *  行落库携带 Image 块（前端展示数据源），其图归工具归属 → assistant 侧
 *  图片卡渲染；此处误判会把 agent 汇报图渲染成右侧「用户发出」气泡。 */
export function hasUserMedia(msg: { content_blocks?: string }): boolean {
  if (!msg.content_blocks || msg.content_blocks === '[]') return false;
  if (rowHasToolResult(msg.content_blocks)) return false;
  return parseImageBlocks(msg.content_blocks).length > 0
    || parseAttachmentBlocks(msg.content_blocks).length > 0;
}

/** 判断 user 消息是否仅含 tool_result（无文本）。
 *  这种消息是工具调用结果，不单独成气泡，其内容并入上一条 assistant 的工具卡片。
 *  ⚠️ 用 some 语义（rowHasToolResult）而非 every：0.9.9 起带图工具回执行行落库
 *  形态 = [tool_result, image]，every 判定漏跳过该数据行 → 切断 assistant 组
 *  连续性（同回合分裂成多个气泡 + 空用户组，批次 Z 修复）。纯 [image] 的用户
 *  发图行不含 tool_result，不受 some 语义影响——勿回退 every 形态。 */
export function isToolResultOnlyUser(msg: { role: string; content: string; content_blocks: string }): boolean {
  if (msg.role !== "user" || msg.content) return false;
  return rowHasToolResult(msg.content_blocks);
}
