// bootProgress 单测：启动加载页真实进度上报（2026-09-16 Phase2 批）。
// 锁四点：null-safe（无 splash 全 no-op，组件测试环境安全）/ 单调钳制不回跳 /
// 首次上报切确定性形态 + 状态行文案写入 / 100% 恰派发一次 icepaw:boot-ready。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { reportBootProgress, bootReady } from "../bootProgress";

function mountSplash() {
  const splash = document.createElement("div");
  splash.id = "boot-splash";
  splash.innerHTML = `<div class="boot-bar"><div class="boot-bar-fill"></div></div><div class="boot-status">正在启动…</div>`;
  document.body.appendChild(splash);
  return splash;
}

describe("bootProgress（启动加载页真实进度）", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    // 模块级 current/readyDispatched 无法重置——用例设计为链式递进不互相打架：
    // 每条用例独立挂新 splash，进度断言只依赖单调性（后到更低值不回退）
  });

  it("无 #boot-splash（splash 已退场/jsdom 组件测试）→ 全 no-op 不抛错", () => {
    expect(() => reportBootProgress(50, "初始化界面")).not.toThrow();
    expect(bootReady()).toBe(false);
  });

  it("首次上报：循环动画切确定性形态（.boot-determinate）+ width + 状态行文案", () => {
    const splash = mountSplash();
    reportBootProgress(15, "加载应用");
    const fill = splash.querySelector<HTMLElement>(".boot-bar-fill")!;
    expect(fill.classList.contains("boot-determinate")).toBe(true);
    expect(fill.style.width).toBe("15%");
    expect(splash.querySelector<HTMLElement>(".boot-status")!.textContent).toBe("加载应用");
  });

  it("单调钳制：乱序低值上报不回跳（只前进）", () => {
    const splash = mountSplash();
    reportBootProgress(60, "加载会话数据");
    reportBootProgress(50, "乱序迟到"); // 迟到的低值被钳住
    const fill = splash.querySelector<HTMLElement>(".boot-bar-fill")!;
    expect(fill.style.width).toBe("60%");
    // 文案仍可更新（label 与进度独立，不因钳制丢信息）
    expect(splash.querySelector<HTMLElement>(".boot-status")!.textContent).toBe("乱序迟到");
  });

  it("100% 恰派发一次 icepaw:boot-ready；bootReady 置位供竞态补查", () => {
    mountSplash();
    const onReady = vi.fn();
    window.addEventListener("icepaw:boot-ready", onReady);
    reportBootProgress(100, "就绪");
    reportBootProgress(100); // 二次 100 不重复派发（main.ts 的 once 监听 + 补查双路安全）
    expect(onReady).toHaveBeenCalledTimes(1);
    expect(bootReady()).toBe(true);
    window.removeEventListener("icepaw:boot-ready", onReady);
  });
});

// ---- 工业制图语言（2026-10-27 DSH 同源升格）：英文阶段行 + 视觉层乱码落定 ----
// 链式约束：模块级 current 已被上方用例推到 100——本组用例按到达值 100 语义断言。

function mountFullSplash() {
  const splash = document.createElement("div");
  splash.id = "boot-splash";
  splash.innerHTML = `<div class="boot-bar"><div class="boot-bar-fill"></div></div>
    <div class="boot-status">正在启动…</div>
    <div class="boot-status-visual" aria-hidden="true">正在启动…</div>
    <div class="boot-sub">BOOT SEQUENCE</div>`;
  document.body.appendChild(splash);
  return splash;
}

describe("bootProgress（工业制图语言）", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("英文阶段行随到达值切换；视觉层乱码流最终精确落定（权威层恒同步）", async () => {
    const splash = mountFullSplash();
    reportBootProgress(100, "就绪");
    expect(splash.querySelector<HTMLElement>(".boot-status")!.textContent).toBe("就绪");
    expect(splash.querySelector<HTMLElement>(".boot-sub")!.textContent).toBe("READY");
    // 乱码流自终止（~380ms 上限），终态与权威层一致
    await new Promise((r) => setTimeout(r, 520));
    expect(splash.querySelector<HTMLElement>(".boot-status-visual")!.textContent).toBe("就绪");
  });

  it("reduced-motion：视觉层直接写入（无乱码流）", () => {
    vi.stubGlobal(
      "matchMedia",
      vi.fn().mockReturnValue({ matches: true }),
    );
    const splash = mountFullSplash();
    reportBootProgress(100, "即刻就绪");
    expect(splash.querySelector<HTMLElement>(".boot-status-visual")!.textContent).toBe("即刻就绪");
  });

  it("测试夹具缺视觉层/英文行（最小 splash）→ no-op 不抛错（既有契约延续）", () => {
    const splash = document.createElement("div");
    splash.id = "boot-splash";
    splash.innerHTML = `<div class="boot-bar"><div class="boot-bar-fill"></div></div><div class="boot-status">x</div>`;
    document.body.appendChild(splash);
    expect(() => reportBootProgress(100, "就绪")).not.toThrow();
    expect(splash.querySelector<HTMLElement>(".boot-status")!.textContent).toBe("就绪");
  });
});
