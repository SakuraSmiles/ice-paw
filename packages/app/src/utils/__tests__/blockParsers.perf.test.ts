// blockParsers.perf — 前端流畅性回归基准（2026-10-26，用户拍板「性能/交互流畅/
// 视觉第一优先级」的工程抓手）
//
// 锁的是 0.9.7 生产实案的病灶形态：生成中每次 messages 更新（轮边界 1-2 次/轮），
// toolResultIndex 重建必须 ∝ 变化行数而非总行数——memo 生效时数千行大会话的
// 重建 ≈ 毫秒级（未变行 Map 键查询，字符串 hash O(1) 缓存）；memo 丢失时全量
// JSON.parse（大会话行含图片 base64 水合）≈ 秒级 → 主线程长任务 → invoke 响应/
// 组件挂载饿死（设置页跳不动/一直加载中的实案形态）。
//
// 阈值取数量级中间值（200ms），CI 慢机器安全：memo 生效路径 <20ms（10 倍裕度），
// 丢失路径秒级（10 倍以上区分度）——回归发生时必然爆红，正常运行永不误报。
import { describe, it, expect } from "vitest";
import { buildToolResultIndex, parseResultRow } from "../blockParsers";

/** 模拟一条带图片水合的 tool_result 行（UE5/view_image 生产形态）：
 *  content_blocks JSON 含一个 tool_result + 一个大 base64 Image 块。 */
function makeUserRow(i: number, base64Size = 200_000): string {
  const b64 = "A".repeat(base64Size); // 模拟水合回内联的图片 base64
  return JSON.stringify([
    { type: "tool_result", tool_use_id: `tu-${i}`, content: `结果 ${i}`, is_error: false },
    { type: "image", data: b64, media_type: "image/png" },
  ]);
}

/** 大会话 fixture：N 行 user（含大 JSON）交替 assistant（轻文本行）。 */
function makeLargeMessages(n: number): { role: string; content_blocks?: string }[] {
  const rows: { role: string; content_blocks?: string }[] = [];
  for (let i = 0; i < n; i++) {
    if (i % 2 === 0) {
      rows.push({ role: "user", content_blocks: makeUserRow(i) });
    } else {
      rows.push({ role: "assistant", content_blocks: JSON.stringify([{ type: "text" }]) });
    }
  }
  return rows;
}

describe("blockParsers 性能基准（流畅性回归锁）", () => {
  it("生成中轮边界更新：变更单行后索引重建 < 200ms（memo 保障 ∝ 变化行非总行数）", () => {
    const N = 2000; // 2000 user 行 × ~200KB ≈ 生产大会话量级（UE5 提取图形态）
    const messages = makeLargeMessages(N * 2);

    // 首次全量建索引（parse 全部行——真实成本，只记录不断言）
    const t0 = performance.now();
    const idx1 = buildToolResultIndex(messages);
    const fullBuildMs = performance.now() - t0;
    expect(idx1.size).toBe(N);

    // 模拟轮边界更新：末条替换为新 tool_result 行（id 用 tu-new 防与 fixture 撞）
    //（ChatMessages 的 messages 是末条整替式更新——引用变、绝大多数行字符串不变）
    const updated = [
      ...messages.slice(0, -1),
      { role: "user", content_blocks: JSON.stringify([
        { type: "tool_result", tool_use_id: "tu-new", content: "新轮结果" },
        { type: "image", data: "B".repeat(50_000), media_type: "image/png" },
      ]) },
    ];

    const t1 = performance.now();
    const idx2 = buildToolResultIndex(updated);
    const rebuildMs = performance.now() - t1;
    expect(idx2.size).toBe(N + 1);

    // 核心回归锁：重建成本必须远小于全量重 parse（数量级）。
    // memo 生效 ≈ Map 键查询 × 4000 行 < 20ms；丢失 = fullBuildMs（秒级）。
    // 200ms 阈值：正常运行 10 倍裕度、回归时 5 倍以上爆红。
    expect(
      rebuildMs,
      `索引重建 ${rebuildMs.toFixed(1)}ms 应 < 200ms（全量首建 ${fullBuildMs.toFixed(1)}ms 作对照；` +
        `若重建接近首建 = 行级 memo 失效 = 0.9.7 主线程饿死回归`,
    ).toBeLessThan(200);
  });

  it("memo 直测：同字符串重复解析恒等返回且 < 15ms（万次级调用面）", () => {
    const row = makeUserRow(0, 500_000);
    parseResultRow(row); // 预热（首次 parse 进缓存）

    const t = performance.now();
    for (let i = 0; i < 10_000; i++) parseResultRow(row);
    const ms = performance.now() - t;
    expect(ms, "万次同串查询应 < 15ms（Map 键 + 字符串 hash 缓存；CI 波动余量）").toBeLessThan(15);
  });

  it("图片归属语义锁：Image 块按序归属最近 tool_result（抽模块不改行为）", () => {
    const row = JSON.stringify([
      { type: "tool_result", tool_use_id: "a", content: "1" },
      { type: "image", data: "IMG_A", media_type: "image/png" },
      { type: "image", data: "IMG_A2", media_type: "image/png" },
      { type: "tool_result", tool_use_id: "b", content: "2", is_error: true },
      { type: "image", data: "IMG_B", media_type: "image/jpeg" },
    ]);
    const entries = parseResultRow(row);
    expect(entries).toHaveLength(2);
    const map = new Map(entries);
    expect(map.get("a")!.images).toEqual([
      { data: "IMG_A", mediaType: "image/png" },
      { data: "IMG_A2", mediaType: "image/png" },
    ]);
    expect(map.get("b")!.images).toEqual([{ data: "IMG_B", mediaType: "image/jpeg" }]);
    expect(map.get("b")!.isError).toBe(true);
    expect(map.get("a")!.isError).toBe(false);
  });
});
