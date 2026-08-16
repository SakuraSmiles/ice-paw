// useActiveTurn 单测：pickActiveTurn 纯判定 + fake IntersectionObserver 接线。
// 背景见 composable 头注释——治 topsCache 静态 offsetTop 在
// content-visibility:auto 估高布局下的系统性漂移。
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
// pickActiveTurn 纯判定
// ---------------------------------------------------------------------------

describe("pickActiveTurn", () => {
  it("空相交集合 → null（视口落在长轮中部，保持上次值）", () => {
    expect(pickActiveTurn([])).toBeNull();
  });

  it("最小轮锚点已过判定线（top ≤ 80）→ 该轮", () => {
    expect(pickActiveTurn([{ turn: 3, top: 80 }])).toBe(3);
    expect(pickActiveTurn([{ turn: 3, top: 0 }])).toBe(3);
  });

  it("最小轮锚点在线下（top > 80）→ 上一轮（在读其尾部）", () => {
    expect(pickActiveTurn([{ turn: 3, top: 200 }])).toBe(2);
  });

  it("t-1 下探钳制到 1（第 1 轮锚点在线下也不得变 0）", () => {
    expect(pickActiveTurn([{ turn: 1, top: 300 }])).toBe(1);
  });

  it("取最小轮判定（多轮相交时 min-turn 主导，其余轮位置无关）", () => {
    // 轮 2 过线、轮 5 也相交：读的是轮 2
    expect(pickActiveTurn([
      { turn: 5, top: 0 },
      { turn: 2, top: 40 },
    ])).toBe(2);
    // 轮 2 未过线 → 轮 1（尽管轮 5 早已过线）
    expect(pickActiveTurn([
      { turn: 5, top: 0 },
      { turn: 2, top: 120 },
    ])).toBe(1);
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

function fireIO(io: FakeIO, entries: { el: Element; isIntersecting: boolean }[]) {
  io.cb(
    entries.map(({ el, isIntersecting }) => ({ target: el, isIntersecting })) as unknown as IntersectionObserverEntry[],
    io as unknown as IntersectionObserver,
  );
}

const nextFrame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));

describe("useActiveTurn 接线", () => {
  beforeEach(() => {
    FakeIO.instances = [];
    vi.stubGlobal("IntersectionObserver", FakeIO as unknown as typeof IntersectionObserver);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  it("只观察有锚点的元素；IO 相交/离场驱动 activeTurn；离场后保持上次值", async () => {
    const container = document.createElement("div");
    const els = [1, 2, 3].map((i) => {
      const el = document.createElement("div");
      el.dataset.mid = `m${i}`;
      container.appendChild(el);
      return el;
    });
    // 无锚点对应的消息组（assistant 等）不得被观察
    const noAnchor = document.createElement("div");
    noAnchor.dataset.mid = "assistant-x";
    container.appendChild(noAnchor);

    const containerRef = ref<HTMLElement | null>(null);
    const anchorsRef = ref<TurnAnchor[]>([anchor(1), anchor(2), anchor(3)]);
    const { result, unmount } = withSetup(() => useActiveTurn(containerRef, anchorsRef));
    containerRef.value = container;
    await nextTick();
    result.refresh();

    const io = FakeIO.instances[FakeIO.instances.length - 1];
    expect(io.root).toBe(container);
    expect(io.observed.size).toBe(3);
    expect(io.observed.has(noAnchor)).toBe(false);

    // 轮 2 相交但锚点在判定线下（top=200）→ 正在读轮 1 尾部
    mockRect(els[1], 200);
    fireIO(io, [{ el: els[1], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(1);

    // 轮 2 锚点滚过判定线 → active = 2
    mockRect(els[1], 40);
    fireIO(io, [{ el: els[1], isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(2);

    // 轮 2 离开视口（视口落在其后内容中）→ 保持 2
    fireIO(io, [{ el: els[1], isIntersecting: false }]);
    expect(result.activeTurn.value).toBe(2);

    unmount();
  });

  it("scroll 只重判边界（rAF 合帧），不依赖 IO 再触发", async () => {
    const container = document.createElement("div");
    const el = document.createElement("div");
    el.dataset.mid = "m1";
    container.appendChild(el);

    const containerRef = ref<HTMLElement | null>(null);
    const anchorsRef = ref<TurnAnchor[]>([anchor(1)]);
    const { result, unmount } = withSetup(() => useActiveTurn(containerRef, anchorsRef));
    containerRef.value = container;
    await nextTick();
    result.refresh();

    const io = FakeIO.instances[FakeIO.instances.length - 1];
    mockRect(el, 40);
    fireIO(io, [{ el, isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(1);

    // 持续相交、无 IO 事件，仅元素滚到判定线下 → scroll 路径更新为上一轮
    mockRect(el, 300);
    container.dispatchEvent(new Event("scroll"));
    await nextFrame();
    expect(result.activeTurn.value).toBe(1);

    unmount();
  });

  it("refresh 重建观察：翻页后新元素入观察、旧实例断开；轮号重映射即时生效", async () => {
    const container = document.createElement("div");
    const el = document.createElement("div");
    el.dataset.mid = "m1";
    container.appendChild(el);

    const containerRef = ref<HTMLElement | null>(null);
    // 翻页场景：m1 原是轮 1，加载更早一页后 m1 变轮 2
    const anchorsRef = ref<TurnAnchor[]>([anchor(1)]);
    const { result, unmount } = withSetup(() => useActiveTurn(containerRef, anchorsRef));
    containerRef.value = container;
    await nextTick();
    result.refresh();

    const io1 = FakeIO.instances[FakeIO.instances.length - 1];
    mockRect(el, 40);
    fireIO(io1, [{ el, isIntersecting: true }]);
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

    // 同一元素同位置，轮号按新映射判：m1 过线 → 轮 2
    fireIO(io2, [{ el, isIntersecting: true }]);
    expect(result.activeTurn.value).toBe(2);

    unmount();
  });
});
