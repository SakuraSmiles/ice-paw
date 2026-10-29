// 生产实案回归（2026-10-29 他机报障）：show_image 的 tool_result 行
// [ToolResult, Image] 同行落库（后端 e2e 证明行含图），前端 parseResultRow
// 必须把 Image 归属到对应 tool_result 条目——空 images = 图片卡不渲染的根因位。
import { describe, it, expect } from "vitest";
import { parseResultRow, buildToolResultIndex, rowHasToolResult } from "../blockParsers";

const row = JSON.stringify([
  { type: "tool_result", tool_use_id: "call_A", content: "{\"path\":\"x.png\",\"note\":\"图片已展示给用户\"}", is_error: false },
  { type: "image", data: "iVBORw0KGgo=", media_type: "image/png" },
  { type: "tool_result", tool_use_id: "call_B", content: "{\"path\":\"y.png\"}", is_error: false },
  { type: "image", data: "QUJD", media_type: "image/png" },
]);

describe("parseResultRow 生产行形状（tool_result+image 交替）", () => {
  it("Image 归属到紧邻在前的 tool_result 条目", () => {
    const entries = parseResultRow(row);
    const a = entries.find(([id]) => id === "call_A")?.[1];
    const b = entries.find(([id]) => id === "call_B")?.[1];
    expect(a?.images.length).toBe(1);
    expect(a?.images[0]?.data).toBe("iVBORw0KGgo=");
    expect(b?.images.length).toBe(1);
    expect(b?.images[0]?.data).toBe("QUJD");
  });

  it("buildToolResultIndex 从 user 行构建全局索引含图", () => {
    const idx = buildToolResultIndex([
      { role: "user", content_blocks: row },
      { role: "assistant", content_blocks: "[]" },
    ]);
    expect(idx.get("call_A")?.images.length).toBe(1);
    expect(idx.get("call_B")?.images.length).toBe(1);
  });

  it("image 在行首（无前导 tool_result）不归属、不炸", () => {
    const lone = JSON.stringify([
      { type: "image", data: "AAA", media_type: "image/png" },
      { type: "tool_result", tool_use_id: "c", content: "x" },
    ]);
    const entries = parseResultRow(lone);
    const c = entries.find(([id]) => id === "c")?.[1];
    expect(c?.images.length).toBe(0);
  });
});

describe("rowHasToolResult（用户媒体判定排除位）", () => {
  it("tool_result+image 数据行 → true（hasUserMedia 据此排除，不渲染成用户气泡）", () => {
    expect(rowHasToolResult(row)).toBe(true);
  });
  it("纯用户上传行 [text, image] → false（正常用户媒体）", () => {
    const upload = JSON.stringify([
      { type: "text", text: "你能看到我发的图片吗" },
      { type: "image", data: "iVBOR", media_type: "image/png" },
    ]);
    expect(rowHasToolResult(upload)).toBe(false);
  });
});
