// useActiveTurn 单测：pickActiveTurn 纯判定（底线语义）+ fake IntersectionObserver
// 接线 + 跳转钉子。背景见 composable 头注释——底线 = 输入框上方一根判定线，
// 线在哪轮的区域里就是哪轮；跳转钉住 N，亲手滚动解钉。
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { pickActiveTurn, useActiveTurn } from "../useActiveTurn";
import type { TurnAnchor } from "../../types";

const anchor = (i: number): TurnAnchor => ({
  message_id: `m${i}`,
  preview: `问题 ${i}`,
  created_at: "2026-08-15 10:00:00",
});

// ---------------------------------------------------------------------------
// pickActiveTurn 纯判定（底线语义：线落哪轮的区域就是哪轮）
// ---------------------------------------------------------------------------

describe("pickActiveTurn", () => {
  it("空相交集合 → null（视口落在长轮中部，保持上次值）", () => {
    expect(pickActiveTurn([], 576)).toBeNull();
  });

  it("线以上有锚点 → 线以上最大轮号（线落在其区域内）", () => {
    expect(pickActiveTurn([{ turn: 3, top: 576 }], 576)).toBe(3);
    expect(pickActiveTurn([{ turn: 3, top: 0 }], 576)).toBe(3);
    // 轮 1、3 都在线上，轮 2 缺席（不相交）→ 线在轮 3 的区域
    expect(pickActiveTurn([{ turn: 1, top: 0 }, { turn: 3, top: 500 }], 576)).toBe(3);
    // 轮 3 锚点在线下 → 线在轮 2 的区域
    expect(pickActiveTurn([{ turn: 1, top: 0 }, { turn: 3, top: 500 }], 400)).toBe(1);
  });

  it("线以上无、线下有 → 线下最小轮 - 1（线在首锚上方，读上一轮尾部）", () => {
    expect(pickActiveTurn([{ turn: 3, top: 200 }], 100)).toBe(2);
    expect(pickActiveTurn([{ turn: 2, top: 300 }, { turn: 3, top: 500 }], 100)).toBe(1);
  });

  it("u-1 下探钳制到 1（线在第 1 轮锚点上方也不得变 0）", () => {
    expect(pickActiveTurn([{ turn: 1, top: 300 }], 100)).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// fake IntersectionObserver 接线（jsdom 无原生 IO，stub 之）
// ---------------------------------------------------------------------------

class FakeIO {
  static instances: FakeIO[] = [];
  cb: IntersectionObserverCallback;
  root: Element | Document | null;
  observed = new Set<Element>();

  constructor(cb: IntersectionObserverCallback, opts?: IntersectionObserverInit) {
    this.cb = cb;
    this.root = opts?.root ?? null;
    FakeIO.instances.push(this);
  }
  observe(el: Element) { this.observed.add(el); }
  unobserve(el: Element) { this.observed.delete(el); }
  disconnect() { this.observed.clear(); }
  takeRecords(): IntersectionObserverEntry[] { return []; }
  get rootMargin(): string { return "0px"; }
  get thresholds(): number[] { return [0]; }
}

/** 以真实组件 setup 跑 composable（watch/onBeforeUnmount 需组件实例） */
function withSetup<T>(composable: () => T): { result: T; unmount: () => void } {
  let result: T | undefined;
  const app = createApp(defineComponent({
    setup() {
      result = composable();
      return () => h("div");
    },
  }));
  const host = document.createElement("div");
  app.mount(host);
  return { result: result!, unmount: () => app.unmount() };
}

/** 元素距视口顶距离固定为 top（jsdom 布局全 0，需 mock gBCR） */
function mockRect(el: HTMLElement, top: number) {
  el.getBoundingClientRect = () => ({ top, bottom: top, height: 0, left: 0, right: 0, width: 0, x: 0, y: top, toJSON: () => ({}) }) as DOMRect;
}

/** 容器高固定为 h（jsdom clientHeight 恒 0；线 Y = h - LINE_FROM_BOTTOM_PX(24)） */
function mockHeight(el: HTMLElement, h: number) {
  Object.defineProperty(el, "clientHeight", { value: h, configurable: true });
}

function fireIO(io: FakeIO, entries: { el: Element; isIntersecting: boolean }[]) {
  io.cb(
    entries.map(({ el, isIntersecting }) => ({ target: el, isIntersecting })) as unknown as IntersectionObserverEntry[],
    io as unknown as IntersectionObserver,
  );
}

const nextFrame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));

/** 标准夹具：容器高 600（线 Y=576），三轮锚点元素 m1/m2/m3 */
function fixture(turns: number) {
  const container = document.createElement("div");
  mockHeight(container, 600);
  const els = Array.from({ length: turns }, (_, i) => {
    const el = document.createElement("div");
    el.dataset.mid = `m${i + 1}`;
    container.appendChild(el);
    return el;
  });
  return { container, els };
}

describe("useActiveTurn 接线（底线语义）", () => {
  beforeEach(() => {
    FakeIO.instances = [];
    vi.stubGlobal("IntersectionObserver", FakeIO as unknown as typeof IntersectionObserver);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  async function setup(turns: number) {
    const { container, els } = fixture(turns);
    const containerRef = ref<HTMLElement | null>(null);
    const anchorsRef = ref<TurnAnchor[]>(Array.from({ length: turns }, (_, i) => anchor(i + 1)));
    const { result, unmount } = withSetup(() => useActiveTurn(containerRef, anchorsRef));
    containerRef.value = container;
    await nextTick();
    result.refresh();
    const io = FakeIO.instances[FakeIO.instances.length - 1];
    return { result, unmount, container, els, io, anchorsRef };
  }

  it("线在哪轮的区域里就是哪轮；离场后保持上次值", async () => {
    const { result, unmount, els, io } = await setup(3);
    // 轮 2 锚点 top=200 ≤ 线 576 → 线在轮 2 区域（顶线旧规则在此给 1）
    mockRect(els[1], 200);
    fireIO(io, [{ el: els[1], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(2);

    // 轮 3 也相交且在线上（top=500）→ 线以上最大轮 = 3
    mockRect(els[2], 500);
    fireIO(io, [{ el: els[2], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(3);

    // 轮 3 离开视口（线落回轮 2 区域尾部）→ 回 2
    fireIO(io, [{ el: els[2], isIntersecting: false }]);
    expect(result.activeTurn.value).toBe(2);

    // 全员离场（线在长轮中部）→ 保持上次值
    fireIO(io, [{ el: els[1], isIntersecting: false }]);
    expect(result.activeTurn.value).toBe(2);

    unmount();
  });

  it("scroll 只重判边界（rAF 合帧）：元素滚过线即换号，不依赖 IO 再触发", async () => {
    const { result, unmount, container, els, io } = await setup(2);
    // 初始：轮 1 在线上（100）、轮 2 在线下（700）→ 线在轮 1 区域
    mockRect(els[0], 100);
    mockRect(els[1], 700);
    fireIO(io, [{ el: els[0], isIntersecting: true }, { el: els[1], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(1);

    // 滚动：轮 2 锚点升到线上（500）→ 线在轮 2 区域（scroll 路径，无 IO 事件）
    mockRect(els[1], 500);
    container.dispatchEvent(new Event("scroll"));
    await nextFrame();
    expect(result.activeTurn.value).toBe(2);

    unmount();
  });

  it("跳转钉子：pin 即时生效压住线判定；滚轮/滚动键解钉；锚点重载失效", async () => {
    const { result, unmount, container, els, io, anchorsRef } = await setup(2);
    mockRect(els[0], 100);
    mockRect(els[1], 700);
    fireIO(io, [{ el: els[0], isIntersecting: true }, { el: els[1], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(1);

    // 跳转钉住轮 5（线判定说 1，钉子赢——跳转后线可能落在 N+k 区域）
    result.pin(5);
    expect(result.activeTurn.value).toBe(5);
    // 钉住期 IO/scroll 重算静默：锚点位置变化不改号
    mockRect(els[1], 300);
    container.dispatchEvent(new Event("scroll"));
    await nextFrame();
    expect(result.activeTurn.value).toBe(5);

    // 滚轮 = 亲手滚动意图 → 解钉，立即回归线判定（轮 2 已在线上）
    container.dispatchEvent(new Event("wheel"));
    expect(result.activeTurn.value).toBe(2);

    // 再钉 + 锚点列表重载（切会话/新轮）→ 钉子失效
    result.pin(5);
    anchorsRef.value = [anchor(1), anchor(2), { ...anchor(3), message_id: "m3" }];
    mockRect(els[1], 700); // 轮 2 回线下 → 线在轮 1 区域
    container.dispatchEvent(new Event("scroll"));
    await nextFrame();
    expect(result.activeTurn.value).toBe(1);

    unmount();
  });

  it("滚动键（键盘滚动）也解钉", async () => {
    const { result, unmount, container, els, io } = await setup(2);
    mockRect(els[0], 100);
    mockRect(els[1], 700);
    fireIO(io, [{ el: els[0], isIntersecting: true }, { el: els[1], isIntersecting: true }]);
    result.pin(3);
    expect(result.activeTurn.value).toBe(3);

    container.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown" }));
    expect(result.activeTurn.value).toBe(1); // 解钉 + 立即线判定

    // 非滚动键不解钉
    result.pin(3);
    container.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
    expect(result.activeTurn.value).toBe(3);

    unmount();
  });

  it("refresh 重建观察：翻页后新元素入观察、旧实例断开；轮号重映射即时生效", async () => {
    const { result, unmount, container, els, io: io1, anchorsRef } = await setup(1);
    mockRect(els[0], 40);
    fireIO(io1, [{ el: els[0], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(1);

    // 加载更早一页：锚点列表前置一条 → m1 变轮 2
    anchorsRef.value = [{ ...anchor(0), message_id: "m0" }, anchor(1)];
    const earlier = document.createElement("div");
    earlier.dataset.mid = "m0";
    container.prepend(earlier);
    result.refresh();

    const io2 = FakeIO.instances[FakeIO.instances.length - 1];
    expect(io2).not.toBe(io1);
    expect(io1.observed.size).toBe(0);
    expect(io2.observed.size).toBe(2);

    // 同一元素同位置，轮号按新映射判：m1 在线上 → 轮 2
    fireIO(io2, [{ el: els[0], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(2);

    unmount();
  });

  it("空集无上次值（切会话贴底长尾轮）→ bootstrap：贴底精确 = 末轮，不横杠", async () => {
    const { result, unmount, els, io } = await setup(3);
    // 场景：末轮回复很长，锚点全滚出视口 → IO 全员报不相交 → 集空
    fireIO(io, [
      { el: els[0], isIntersecting: false },
      { el: els[1], isIntersecting: false },
      { el: els[2], isIntersecting: false },
    ]);
    // jsdom 滚动几何全 0：scrollTop+clientHeight(0) ≥ scrollHeight(0)-4 → 贴底
    expect(result.activeTurn.value).toBe(3); // 末轮，不是 null/横杠

    unmount();
  });

  it("锚点重载即失忆：上次值不跨界存活（切会话错号来源）", async () => {
    const { result, unmount, els, io, anchorsRef } = await setup(3);
    mockRect(els[0], 100);
    fireIO(io, [{ el: els[0], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(1);

    // 切会话：锚点列表整体替换（新界只有 2 轮）→ activeTurn 失忆（watch 是
    // pre-flush，赋值后须 nextTick 才回调）
    anchorsRef.value = [anchor(1), { ...anchor(2), message_id: "new-2" }];
    await nextTick();
    expect(result.activeTurn.value).toBeNull();

    unmount();
  });

  it("DOM 换血竞态：旧元素移出容器 + 锚点重载失忆 → 空集 bootstrap，不出旧轮号", async () => {
    const { result, unmount, container, els, io, anchorsRef } = await setup(3);
    mockRect(els[0], 100);
    fireIO(io, [{ el: els[0], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(1);

    // 竞态窗：切会话后元素已移出容器（gBCR 将全 0）、锚点已重载（失忆），
    // refresh 重建前的这次滚动重算——剔除旧元素、空集走 bootstrap
    els[0].remove();
    anchorsRef.value = [anchor(1), anchor(2), anchor(3)];
    await nextTick();
    container.dispatchEvent(new Event("scroll"));
    await nextFrame();
    // 不读 0 坐标算出旧轮 1；bootstrap（jsdom 全 0 几何 = 贴底）→ 末轮
    expect(result.activeTurn.value).toBe(3);

    unmount();
  });
});
