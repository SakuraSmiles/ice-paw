// useScrollFollow 纯函数单测：滚动恢复决策（DOM 交互部分靠真机手测覆盖）
import { describe, it, expect } from "vitest";
import { planScrollRestore, type ScrollAnchor } from "../useScrollFollow";

const ids = (arr: string[]) => new Set(arr);

describe("planScrollRestore", () => {
  it("无锚点（首次打开/从未滚动）→ 贴底", () => {
    expect(planScrollRestore(undefined, ids(["m1", "m2"]))).toBe("bottom");
  });

  it("离开时在底部（跟随态）→ 贴底，哪怕有旧锚点消息 id", () => {
    const a: ScrollAnchor = { messageId: "m1", offset: 10, atBottom: true };
    expect(planScrollRestore(a, ids(["m1", "m2"]))).toBe("bottom");
  });

  it("读历史 + 锚点在已加载窗口 → 原位恢复", () => {
    const a: ScrollAnchor = { messageId: "m2", offset: 32, atBottom: false };
    expect(planScrollRestore(a, ids(["m1", "m2", "m3"]))).toBe("restore");
  });

  it("读历史 + 锚点在分页窗口外（只加载了最新 50 条）→ 先翻页", () => {
    const a: ScrollAnchor = { messageId: "m-old", offset: 0, atBottom: false };
    expect(planScrollRestore(a, ids(["m40", "m41", "m42"]))).toBe("paginate");
  });

  it("显式贴底写入的锚点（messageId 空）→ 贴底", () => {
    // scrollToBottom 直接改写锚点意图（suppress 窗口内滚动事件不采样），
    // 不写会残留「读历史」旧锚点 → 回来时错误原位恢复
    const a: ScrollAnchor = { messageId: "", offset: 0, atBottom: true };
    expect(planScrollRestore(a, ids(["m1"]))).toBe("bottom");
  });
});
