// useScrollFollow 纯函数单测：滚动恢复决策（DOM 交互部分靠真机手测覆盖）
import { describe, it, expect } from "vitest";
import {
  planScrollRestore,
  computePrependRestore,
  shouldTriggerPrepend,
  shouldEdgeTriggerPrepend,
  shouldContinuePrependChain,
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

describe("shouldContinuePrependChain（净高续拉）", () => {
  it("链内累计净增不足一屏且还有更早 → 续拉（长工具回合整页并进折叠组 ≈0 净增）", () => {
    // 分页计量单位是 messages 表行（50 行/页）而非渲染内容——一整页可以
    // 只渲染出一条胶囊行；一次上滚至少带出一屏可读内容，不靠连续 wheel 凑
    expect(shouldContinuePrependChain({ netHeight: 0, viewportHeight: 800, pagesLoaded: 1, productivePages: 0, hasMore: true })).toBe(true);
    expect(shouldContinuePrependChain({ netHeight: 300, viewportHeight: 800, pagesLoaded: 2, productivePages: 1, hasMore: true })).toBe(true);
  });

  it("链内累计净增 ≥ 一屏 → 停（已带出足够可读内容）", () => {
    expect(shouldContinuePrependChain({ netHeight: 800, viewportHeight: 800, pagesLoaded: 1, productivePages: 1, hasMore: true })).toBe(false);
    expect(shouldContinuePrependChain({ netHeight: 1200, viewportHeight: 800, pagesLoaded: 3, productivePages: 2, hasMore: true })).toBe(false);
  });

  it("「带出可见高度」的页达上限（5 页）→ 停（防多屏可见内容段一次手势连拉）", () => {
    expect(shouldContinuePrependChain({ netHeight: 0, viewportHeight: 800, pagesLoaded: 5, productivePages: 5, hasMore: true })).toBe(false);
  });

  it("整页并组页不计数：净增≈0 的页连拉 8 页仍续拉（八轮拍板「链拉到可见内容为止」）", () => {
    // 「默认收起」拍板下整页并组页结构性不可见、滚动条物理上不能动——
    // 并组页不占 5 页可见内容上限，链继续拉到回合边界（可见内容出现）才停
    expect(shouldContinuePrependChain({ netHeight: 0, viewportHeight: 800, pagesLoaded: 8, productivePages: 0, hasMore: true })).toBe(true);
  });

  it("总页数硬上限（20 页）→ 停（并组段再长也有界，500+ 行回合一次手势不全拉）", () => {
    expect(shouldContinuePrependChain({ netHeight: 0, viewportHeight: 800, pagesLoaded: 20, productivePages: 0, hasMore: true })).toBe(false);
  });

  it("没有更早消息 → 停（加载尽即止，不空转）", () => {
    expect(shouldContinuePrependChain({ netHeight: 0, viewportHeight: 800, pagesLoaded: 1, productivePages: 0, hasMore: false })).toBe(false);
  });
});

describe("forceRealizeAbove / clearForcedRealize（恢复前强制实测）", () => {
  // content-visibility 估高毒（2026-09-16 五轮）：offsetTop 量取必须在真实
  // 高度下进行。jsdom 不做布局，此处锁内联样式的置/清/钉契约（真值几何靠真机）
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
  /** 桩实测高度（jsdom 无布局，offsetHeight 恒 0） */
  function stubHeights(groups: HTMLElement[], heights: number[]) {
    groups.forEach((g, i) =>
      Object.defineProperty(g, "offsetHeight", { configurable: true, value: heights[i] }));
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

  it("清除时把实测高度钉进内联 contain-intrinsic-size（七轮：auto 记忆在强制窗口内不保证触发，清除即钉高防回弹）", () => {
    const { root, groups } = fixture();
    stubHeights(groups, [680, 720, 700, 800]);
    const forced = forceRealizeAbove(root, groups[3]);
    clearForcedRealize(forced);
    // content-visibility 内联全撤（组回跳过态、虚拟化省耗保留）
    for (const g of groups) {
      expect(g.style.getPropertyValue("content-visibility")).toBe("");
    }
    // 占位高度 = 实测真值（保留 auto 前缀——组此后真实入画浏览器记忆自行接管）
    expect(groups[0].style.getPropertyValue("contain-intrinsic-size")).toBe("auto 680px");
    expect(groups[1].style.getPropertyValue("contain-intrinsic-size")).toBe("auto 720px");
    expect(groups[2].style.getPropertyValue("contain-intrinsic-size")).toBe("auto 700px");
    expect(groups[3].style.getPropertyValue("contain-intrinsic-size")).toBe("");
  });

  it("量不出高度（jsdom 无布局/极端未渲染 offsetHeight=0）→ 不写 0 值钉高", () => {
    const { root, groups } = fixture();
    stubHeights(groups, [0, 0, 0, 0]);
    const forced = forceRealizeAbove(root, groups[2]);
    clearForcedRealize(forced);
    expect(groups[0].style.getPropertyValue("contain-intrinsic-size")).toBe("");
    expect(groups[1].style.getPropertyValue("contain-intrinsic-size")).toBe("");
    expect(groups[0].style.getPropertyValue("content-visibility")).toBe("");
  });

  it("锚是首组 → 零强制零成本（上方无屏外组）", () => {
    const { root, groups } = fixture();
    expect(forceRealizeAbove(root, groups[0])).toEqual([]);
    expect(groups[0].style.getPropertyValue("content-visibility")).toBe("");
  });
});
