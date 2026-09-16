// useScrollFollow 纯函数单测：滚动恢复决策（DOM 交互部分靠真机手测覆盖）
import { describe, it, expect } from "vitest";
import {
  planScrollRestore,
  computePrependRestore,
  shouldTriggerPrepend,
  shouldEdgeTriggerPrepend,
  forceRealizeAbove,
  clearForcedRealize,
  type ScrollAnchor,
} from "../useScrollFollow";

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

describe("shouldTriggerPrepend（前插分页方向闸）", () => {
  it("触发区内向上滚（delta<0）→ 触发（正常翻历史路径不变）", () => {
    expect(shouldTriggerPrepend(180, 400)).toBe(true);
    expect(shouldTriggerPrepend(0, 20)).toBe(true);
  });

  it("触发区内向下轻滚 → 不触发（顶部吸附根治：向下=想离开历史区）", () => {
    // 折叠并组使新页高度≈0、恢复后停在触发区——旧逻辑仅凭位置即再拉一页
    expect(shouldTriggerPrepend(40, 0)).toBe(false);
    expect(shouldTriggerPrepend(150, 90)).toBe(false);
  });

  it("同位重复事件（惯性滚到 0 后 scrollTop 不再变）→ 不触发", () => {
    expect(shouldTriggerPrepend(0, 0)).toBe(false);
    expect(shouldTriggerPrepend(150, 150)).toBe(false);
  });

  it("触发区外（≥200px）不触发，方向无关", () => {
    expect(shouldTriggerPrepend(250, 400)).toBe(false);
    expect(shouldTriggerPrepend(200, 500)).toBe(false); // 边界值=区外
  });
});

describe("shouldEdgeTriggerPrepend（wheel 绝对顶边缘触发）", () => {
  it("绝对顶继续上滚（scrollTop=0 + deltaY<0）→ 触发（连续上滚根治）", () => {
    expect(shouldEdgeTriggerPrepend(0, -120)).toBe(true);
    expect(shouldEdgeTriggerPrepend(0, -3)).toBe(true); // 触控板小步滚
  });

  it("绝对顶向下滚（deltaY>0 / =0）→ 不触发（0 是触发区内、向下归方向闸管）", () => {
    expect(shouldEdgeTriggerPrepend(0, 100)).toBe(false);
    expect(shouldEdgeTriggerPrepend(0, 0)).toBe(false); // 横向滚（shift+wheel）
  });

  it("非绝对顶（scrollTop>0）→ 不触发（1-199 触发区由 scroll 事件路径管辖）", () => {
    // 并组折叠态恢复后常停在触发区内——此处的加载仍走 scroll 方向闸，
    // wheel 边缘只接管 scroll 事件物理上不再产生的 0 这一点位
    expect(shouldEdgeTriggerPrepend(40, -120)).toBe(false);
    expect(shouldEdgeTriggerPrepend(150, -120)).toBe(false);
  });
});

describe("forceRealizeAbove / clearForcedRealize（恢复前强制实测）", () => {
  // content-visibility 估高毒（2026-09-16 五轮）：offsetTop 量取必须在真实
  // 高度下进行。jsdom 不做布局，此处锁内联样式的置/清契约（真值几何靠真机）
  function fixture() {
    const root = document.createElement("div");
    const groups = [1, 2, 3, 4].map((i) => {
      const g = document.createElement("div");
      g.className = "message-group";
      g.dataset.mid = `m${i}`;
      root.appendChild(g);
      return g;
    });
    return { root, groups };
  }

  it("只强制锚之前的组（锚自身与其后的组不动），命中即停", () => {
    const { root, groups } = fixture();
    const forced = forceRealizeAbove(root, groups[2]);
    expect(forced).toEqual([groups[0], groups[1]]);
    expect(groups[0].style.getPropertyValue("content-visibility")).toBe("visible");
    expect(groups[1].style.getPropertyValue("content-visibility")).toBe("visible");
    expect(groups[2].style.getPropertyValue("content-visibility")).toBe("");
    expect(groups[3].style.getPropertyValue("content-visibility")).toBe("");
  });

  it("清除后内联覆盖全部移除（组回估高跳过态，记忆高度接管几何）", () => {
    const { root, groups } = fixture();
    const forced = forceRealizeAbove(root, groups[3]);
    clearForcedRealize(forced);
    for (const g of groups) {
      expect(g.style.getPropertyValue("content-visibility")).toBe("");
    }
  });

  it("锚是首组 → 零强制零成本（上方无屏外组）", () => {
    const { root, groups } = fixture();
    expect(forceRealizeAbove(root, groups[0])).toEqual([]);
    expect(groups[0].style.getPropertyValue("content-visibility")).toBe("");
  });
});
