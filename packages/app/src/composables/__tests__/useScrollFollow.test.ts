// useScrollFollow 纯函数单测：滚动恢复决策（DOM 交互部分靠真机手测覆盖）
import { describe, it, expect } from "vitest";
import { planScrollRestore, computePrependRestore, type ScrollAnchor } from "../useScrollFollow";

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

describe("computePrependRestore（前插分页恢复定位）", () => {
  const base = {
    primaryViewportOffset: 40,
    fallbackViewportOffset: 120,
    prevScrollTop: 150,
    heightDelta: 2000,
  };

  it("主锚在场：锚元素新 offsetTop - 距视口顶偏移（用户读到的内容不动）", () => {
    // 前插后主锚组整体下移 2000 → 复位后仍压在视口原位
    expect(computePrependRestore({
      ...base, primaryOffsetTop: 2040, fallbackOffsetTop: 2200,
    })).toBe(2000);
  });

  it("主锚被并组吞掉（组头易主 data-mid 变化）→ 回退次组锚", () => {
    expect(computePrependRestore({
      ...base, primaryOffsetTop: null, fallbackOffsetTop: 2120,
    })).toBe(2000); // 2120 - 120
  });

  it("主锚次锚皆失（极端：DOM 全重建）→ 高度差兜底（旧策略）", () => {
    expect(computePrependRestore({
      ...base, primaryOffsetTop: null, fallbackOffsetTop: null,
    })).toBe(2150); // 150 + 2000
  });

  it("复位值钳制非负：锚在视口顶上方压线时不回弹负 scrollTop", () => {
    expect(computePrependRestore({
      ...base, primaryOffsetTop: 10, fallbackOffsetTop: 100,
    })).toBe(0);
  });
});
