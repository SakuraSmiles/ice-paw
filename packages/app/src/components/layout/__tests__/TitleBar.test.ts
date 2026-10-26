// TitleBar.test.ts — 自绘窗口栏行为锁（感官升级批 3 · L3）：
// ① 三键齐全带 aria-label（无障碍基线：键盘可达 + 屏幕阅读器命名）；
// ② 最大化态图标切换（Square ↔ Copy）；③ 拖拽区属性在位（栏体承接
// data-tauri-drag-region，Tauri 注入脚本按此判定拖拽/双击最大化）；
// ④ 无 Tauri runtime（happy-dom）下渲染不炸（getCurrentWindow 守卫）。
import { describe, it, expect, afterEach, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";

const mocks = vi.hoisted(() => ({
  minimized: 0,
  toggled: 0,
  closed: 0,
  maximized: false,
}));

// happy-dom 无 Tauri runtime：mock @tauri-apps/api/window（组件侧本就有
// try-catch 静默防御，mock 让三键点击路径可断言调用计数）
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: async () => { mocks.minimized += 1; },
    toggleMaximize: async () => { mocks.toggled += 1; },
    close: async () => { mocks.closed += 1; },
    isMaximized: async () => mocks.maximized,
    onResized: async () => () => {},
    onFocusChanged: async () => () => {},
  }),
}));

const wrappers: VueWrapper[] = [];

async function mountBar() {
  const { default: TitleBar } = await import("../TitleBar.vue");
  const w = mount(TitleBar, { attachTo: document.body });
  wrappers.push(w);
  await new Promise((r) => setTimeout(r, 0)); // 等 onMounted 的 isMaximized 回填
  return w;
}

describe("TitleBar 自绘窗口栏", () => {
  afterEach(() => {
    for (const w of wrappers) w.unmount();
    wrappers.length = 0;
    document.body.innerHTML = "";
    vi.clearAllMocks();
    mocks.minimized = 0;
    mocks.toggled = 0;
    mocks.closed = 0;
    mocks.maximized = false;
  });

  it("三键齐全带 aria-label + 栏体是拖拽区", async () => {
    const w = await mountBar();
    const bar = w.find(".titlebar");
    expect(bar.attributes("data-tauri-drag-region")).toBeDefined();
    const labels = w.findAll(".tb-btn").map((b) => b.attributes("aria-label"));
    expect(labels).toEqual(["最小化", "最大化", "关闭"]);
  });

  it("三键调用对应窗口操作", async () => {
    const w = await mountBar();
    await w.findAll(".tb-btn")[0].trigger("click");
    await w.findAll(".tb-btn")[1].trigger("click");
    await w.findAll(".tb-btn")[2].trigger("click");
    expect(mocks.minimized).toBe(1);
    expect(mocks.toggled).toBe(1);
    expect(mocks.closed).toBe(1);
  });

  it("最大化态图标与 aria 切换（Square→Copy / 最大化→还原）", async () => {
    mocks.maximized = true;
    const w = await mountBar();
    const maxBtn = w.findAll(".tb-btn")[1];
    expect(maxBtn.attributes("aria-label")).toBe("还原");
    // lucide 图标类名锁（组件无 name 元数据，仓内既有断言手法）
    expect(maxBtn.find("svg.lucide-copy").exists()).toBe(true);
    expect(maxBtn.find("svg.lucide-square").exists()).toBe(false);
  });

  it("无拖拽区属性在按钮上（点击不被拖拽吞掉）", async () => {
    const w = await mountBar();
    for (const b of w.findAll(".tb-btn")) {
      expect(b.attributes("data-tauri-drag-region")).toBeUndefined();
    }
  });
});
