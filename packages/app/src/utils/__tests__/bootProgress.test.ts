// bootProgress 单测：启动加载页真实进度上报（2026-09-16 Phase2 批）。
// 锁四点：null-safe（无 splash 全 no-op，组件测试环境安全）/ 单调钳制不回跳 /
// 首次上报切确定性形态 + 状态行文案写入 / 100% 恰派发一次 icepaw:boot-ready。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { reportBootProgress, bootReady, rosterAppend, onRosterDone, rosterRemainingMs } from "../bootProgress";

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

// ---- 就绪名册三组（rosterAppend）：顺序编排 + 状态行 加载态→结果态 切换 ----
// 2026-10-27 顺序编排：agents→projects→sessions 演出时间轴**串行全完成**（下一组
// 在上一组「换字遮罩解除 = 演出彻底完成」+ 停顿后起跑），不是三组同时/仅入场
// 串行；agents 恒为序列头在此重置舞台时钟。
// 时序（STEP=80/IN_TO_REVEAL=200/HOLD=250/EXIT_STEP=60/GROUP_GAP=120）：
//   agents(2行)   起点 0，statusAt 1080、演完 1400 → projects 起点 1520；
//   projects(3行) 起点 1520，statusAt 2740、演完 3060 → sessions 起点 3180；
//   sessions(2行) 起点 3180，statusAt 4260、换字 4460。
describe("bootProgress.rosterAppend（就绪名册三组·顺序编排）", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  function mountGroups() {
    const splash = document.createElement("div");
    splash.id = "boot-splash";
    splash.innerHTML = `<div class="boot-center"></div><div class="boot-roster" aria-hidden="true">
      <div class="roster-group" id="roster-agents">
        <div class="roster-status"><span class="status-text">正在唤醒智能体...</span></div>
        <div class="roster-title">AGENT</div>
      </div>
      <div class="roster-group" id="roster-projects">
        <div class="roster-status"><span class="status-text">正在加载项目空间...</span></div>
        <div class="roster-title">PROJECTS</div>
      </div>
      <div class="roster-group" id="roster-sessions">
        <div class="roster-status"><span class="status-text">正在恢复会话数据...</span></div>
        <div class="roster-title">SESSIONS</div>
      </div>
    </div>`;
    document.body.appendChild(splash);
    return splash;
  }
  const on = (splash: HTMLElement, g: string) =>
    splash.querySelector<HTMLElement>(`#roster-${g} .roster-status`)!.classList.contains("on");
  const textOf = (splash: HTMLElement, g: string) =>
    splash.querySelector<HTMLElement>(`#roster-${g} .status-text`)!.textContent;

  it("三组顺序点亮：agents 立即、projects/sessions 按入场时长串行延迟", () => {
    const splash = mountGroups();
    rosterAppend("agents", ["a1", "a2"]);
    rosterAppend("projects", ["p1", "p2", "p3"]);
    rosterAppend("sessions", ["s1", "s2"]);
    vi.advanceTimersByTime(1); // 落 t=0 的 0ms 定时器（agents 状态行 + 标题点亮）
    expect(on(splash, "agents")).toBe(true);
    expect(splash.querySelector<HTMLElement>("#roster-agents")!.classList.contains("title-on")).toBe(true);
    expect(on(splash, "projects")).toBe(false);
    expect(on(splash, "sessions")).toBe(false);
    // projects 起点 1520（agents 全序列演完 1400 + 停顿 120；下一组等上一组彻底演完）
    vi.advanceTimersByTime(1519);
    expect(on(splash, "projects")).toBe(true);
    expect(on(splash, "sessions")).toBe(false);
    // sessions 起点 3180（projects 全序列演完 3060 + 停顿 120）
    vi.advanceTimersByTime(1660);
    expect(on(splash, "sessions")).toBe(true);
  });

  it("三组结果文案按组配置（计数/固定），done 持久、标题保留、行占位移除", () => {
    const splash = mountGroups();
    rosterAppend("agents", ["a1", "a2"]);
    rosterAppend("projects", ["p1", "p2", "p3"]);
    rosterAppend("sessions", ["s1", "s2"]);
    // 推进到 sessions 换字完成（4460 = 起点 3180 + statusAt 1080 + cover 200）；
    // 三组均已越过各自 statusAt+200：done 图标持久、swapping 遮罩已解除
    vi.advanceTimersByTime(4460);
    expect(textOf(splash, "agents")).toBe("2 名智能体已上线");
    expect(textOf(splash, "projects")).toBe("3 项目解析完成");
    expect(textOf(splash, "sessions")).toBe("会话数据恢复完成");
    const agentsStatus = splash.querySelector<HTMLElement>("#roster-agents .roster-status")!;
    expect(agentsStatus.classList.contains("done")).toBe(true);
    expect(agentsStatus.classList.contains("swapping")).toBe(false);
    // 标题保留为常驻标签（title-on 点亮），行占位一次性移除（collapsed 类，CSS 侧
    // display:none 生效于真实样式表；jsdom 不加载样式表，此处只锁类标记）
    const agentsGroup = splash.querySelector<HTMLElement>("#roster-agents")!;
    expect(agentsGroup.classList.contains("title-on")).toBe(true);
    expect(agentsGroup.classList.contains("collapsed")).toBe(true);
  });

  it("无 splash/未知组容器 → 静默 no-op 不抛错", () => {
    expect(() => rosterAppend("agents", ["x"])).not.toThrow();
    expect(() => rosterAppend("unknown", ["x"])).not.toThrow();
  });

  it("名册全部演完后中心主内容区触发欢迎收尾（.welcoming）", () => {
    const splash = mountGroups();
    rosterAppend("agents", ["a1", "a2"]);
    rosterAppend("projects", ["p1", "p2", "p3"]);
    rosterAppend("sessions", ["s1", "s2"]);
    const center = splash.querySelector<HTMLElement>(".boot-center")!;
    // sessions 演完（groupEnd=4580）+ 停顿 500 前不触发
    vi.advanceTimersByTime(5000);
    expect(center.classList.contains("welcoming")).toBe(false);
    // 越过「演完 4580 + 停顿 500 = 5080」触发欢迎收尾（名册先、欢迎后）
    vi.advanceTimersByTime(200);
    expect(center.classList.contains("welcoming")).toBe(true);
  });

  it("组间 await 真实耗时计入演完时刻：收尾不早于最后一组真正演完", () => {
    const splash = mountGroups();
    // 模拟 Sidebar 三组之间 await（project.load / chat.loadConversations）的真实耗时：
    // performance.now 逐组前移 500/700ms。旧实现用「序列头基准 + 累计偏移」漏算这段
    // 等待 → 名册演完时刻被低估、中心「欢迎使用」提前闪现（总/子先后关系颠倒）。
    let now = 1000;
    const perf = vi.spyOn(performance, "now").mockImplementation(() => now);
    rosterAppend("agents", ["a1", "a2"]);
    now += 500;
    rosterAppend("projects", ["p1", "p2", "p3"]);
    now += 700;
    rosterAppend("sessions", ["s1", "s2"]);
    perf.mockRestore();
    const center = splash.querySelector<HTMLElement>(".boot-center")!;
    // sessions 相对演完 4580（组间 1200 等待已折算进绝对时刻）+ 停顿 500；推进 5000 尚未演完
    vi.advanceTimersByTime(5000);
    expect(center.classList.contains("welcoming")).toBe(false);
    vi.advanceTimersByTime(200);
    expect(center.classList.contains("welcoming")).toBe(true);
  });

  it("onRosterDone 回调在名册最后一组演完时触发（就绪 100% 推迟到名册之后）", () => {
    mountGroups();
    let now = 1000;
    const perf = vi.spyOn(performance, "now").mockImplementation(() => now);
    const onDone = vi.fn();
    onRosterDone(onDone);
    rosterAppend("agents", ["a1", "a2"]);
    now += 500;
    rosterAppend("projects", ["p1", "p2", "p3"]);
    now += 700;
    rosterAppend("sessions", ["s1", "s2"]);
    perf.mockRestore();
    // sessions 演完时刻 = 2200 + 4580 = 6780；回调定时器 delay = 6780 - 2200 = 4580
    vi.advanceTimersByTime(4500);
    expect(onDone).not.toHaveBeenCalled();
    vi.advanceTimersByTime(200); // 4700 > 4580
    expect(onDone).toHaveBeenCalledTimes(1);
    onRosterDone(() => {}); // 单槽清理（模块级状态跨用例隔离）
  });

  it("rosterRemainingMs 计入停顿 + 欢迎揭示 + 欢迎后停留", () => {
    mountGroups();
    let now = 1000;
    const perf = vi.spyOn(performance, "now").mockImplementation(() => now);
    rosterAppend("agents", ["a1", "a2"]);
    now += 500;
    rosterAppend("projects", ["p1", "p2", "p3"]);
    now += 700;
    rosterAppend("sessions", ["s1", "s2"]);
    // sessions 演完时刻 = 2200 + 4580 = 6780
    now = 6780; // 名册刚演完
    expect(rosterRemainingMs()).toBe(2200); // 停顿 500 + 换字演出 500 + 停留 1200（2026-10-28 提速 + 停留拍板 1.2s）
    now = 6780 + 500; // 欢迎起演
    expect(rosterRemainingMs()).toBe(1700); // 换字演出 500 + 停留 1200
    now = 6780 + 500 + 500; // 换字演完
    expect(rosterRemainingMs()).toBe(1200); // 仅欢迎后停留
    perf.mockRestore();
  });
});
