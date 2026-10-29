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
