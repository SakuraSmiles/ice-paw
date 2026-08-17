// refs.test.ts — @ 引用辅助：短码稳定性 / Reference 块解析 / 组首回溯。
import { describe, it, expect } from "vitest";
import { shortCode, parseReferenceBlocks, resolveGroupMid } from "../refs";

describe("shortCode", () => {
  it("同 id 稳定输出 4 位数字（前导零补齐）", () => {
    expect(shortCode("c2")).toMatch(/^\d{4}$/);
    expect(shortCode("c2")).toBe(shortCode("c2"));
    expect(shortCode("09488f1a-1111-4c2e-9d3f-deadbeef0000")).toMatch(/^\d{4}$/);
  });

  it("不同 id 短码分布（非恒同值）", () => {
    const codes = new Set(Array.from({ length: 50 }, (_, i) => shortCode("id-" + i)));
    expect(codes.size).toBeGreaterThan(10);
  });
});

describe("parseReferenceBlocks", () => {
  it("解析 reference 块并映射字段；空/坏 JSON/无引用返回空", () => {
    const blocks = JSON.stringify([
      { type: "text", text: "看看" },
      { type: "reference", ref_kind: "conversation", target_id: "c2", display: "设计讨论#1234" },
      { type: "attachment", name: "a.pdf", kind: "pdf", size: 1 },
    ]);
    const refs = parseReferenceBlocks(blocks);
    expect(refs).toEqual([
      { refKind: "conversation", targetId: "c2", display: "设计讨论#1234" },
    ]);

    expect(parseReferenceBlocks("[]")).toEqual([]);
    expect(parseReferenceBlocks(null)).toEqual([]);
    expect(parseReferenceBlocks("{bad json")).toEqual([]);
  });
});

describe("resolveGroupMid", () => {
  const msgs = [
    { id: "u1", role: "user" },
    { id: "a1", role: "assistant" },
    { id: "a2", role: "assistant" },
    { id: "u2", role: "user" },
  ];

  it("组中任意消息回溯到组首（assistant 连续组 / user 单条）", () => {
    expect(resolveGroupMid(msgs, "a2")).toBe("a1"); // 组中 → 组首
    expect(resolveGroupMid(msgs, "a1")).toBe("a1"); // 组首原样
    expect(resolveGroupMid(msgs, "u2")).toBe("u2");
  });

  it("未命中 / 空列表返回原 id（调用方 data-mid 直查兜底）", () => {
    expect(resolveGroupMid(msgs, "nope")).toBe("nope");
    expect(resolveGroupMid([], "x")).toBe("x");
  });
});
