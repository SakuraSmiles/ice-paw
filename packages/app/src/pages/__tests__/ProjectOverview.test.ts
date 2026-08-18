// ProjectOverview.test.ts — 概览 tab 二轮重设计后的行为锁定：
// 统计带数字渲染（overview + 成员数）/ 任务状态条分桶 / 最近活动 /
// 成员卡直达设置 / 空态。会话入口已撤（完整列表在侧栏，不重复造）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ProjectOverview from "../project/ProjectOverview.vue";
import { useProjectStore } from "../../stores/project";
import type { Project } from "../../types";

const mockInvoke = vi.mocked(invoke);
const push = vi.fn();

vi.mock("vue-router", () => ({
  useRoute: () => ({ params: { id: "p1" } }),
  useRouter: () => ({ push }),
}));

function project(): Project {
  return {
    id: "p1", name: "Alpha", description: "", icon: "folder", sort_order: 0,
    workspace_path: null, theme_color: null, archived: false,
    created_at: "2026-08-18 00:00:00", updated_at: "2026-08-18 00:00:00",
    agents: [
      { project_id: "p1", agent_id: "a1", role: "member", joined_at: "2026-08-18 00:00:00" },
      { project_id: "p1", agent_id: "a2", role: "member", joined_at: "2026-08-18 00:00:00" },
    ],
  };
}

const OVERVIEW_OUT = {
  chat_conversations: 7, delegation_conversations: 3, messages: 42,
  tasks_total: 3, tasks_done: 1, tasks_failed: 1, tasks_ended_other: 0,
  last_activity_at: "2026-08-18 01:00:00",
  agent_shares: [
    { agent_id: "a1", messages: 30, tokens: 9000 },
    { agent_id: "a2", messages: 12, tokens: 2700 },
  ],
};

function mockBackend(tasks: unknown[] = [], overview = OVERVIEW_OUT) {
  mockInvoke.mockImplementation((async (cmd: string) => {
    switch (cmd) {
      case "get_project_overview": return overview;
      case "list_project_tasks": return tasks;
      case "list_agents": return [
        { id: "a1", name: "前端专家", model: "glm-5.2" },
        { id: "a2", name: "测试专家", model: "minimax-m3" },
        ...Array.from({ length: 6 }, (_, i) => ({
          id: `x${i}`, name: `成员${i}`, model: "m-x",
        })),
      ];
      default: return undefined;
    }
  }) as never);
}

async function mountOverview() {
  const ps = useProjectStore();
  ps.list = [project()];
  ps.loaded = true;
  const w = mount(ProjectOverview);
  await flushPromises();
  return w;
}

describe("ProjectOverview 概览 tab（二轮重设计）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    push.mockReset();
    mockBackend();
  });

  it("统计带：四卡数字（会话/委派任务/消息/成员）来自 overview 与 store", async () => {
    const w = await mountOverview();
    const values = w.findAll(".stat-value").map((n) => n.text());
    expect(values).toEqual(["7", "3", "42", "2"]);
    const labels = w.findAll(".stat-label").map((n) => n.text());
    expect(labels).toEqual(["会话", "委派任务", "消息", "成员"]);
  });

  it("任务状态卡：桶数渲染 + 最近活动；零任务走优雅空态", async () => {
    let w = await mountOverview();
    expect(w.find(".mix-empty").exists()).toBe(true);
    // 空任务也有 meta 行（共 0 个任务 / 最近活动来自 overview）
    expect(w.find(".mix-meta").text()).toContain("共 0 个任务");
    expect(w.find(".mix-meta").text()).toContain("最近活动");

    mockBackend([
      { conv_id: "t1", termination: "stop" },
      { conv_id: "t2", termination: "error" },
    ]);
    w = await mountOverview();
    const legend = w.findAll(".mix-item").map((n) => n.text());
    expect(legend).toContain("已完成 1");
    expect(legend).toContain("未成功 1");
    expect(w.find(".mix-empty").exists()).toBe(false);
    expect(w.find(".mix-meta").text()).toContain("共 2 个任务");
  });

  it("成员卡点击直达设置 tab", async () => {
    const w = await mountOverview();
    await w.find(".stat-card-click").trigger("click");
    expect(push).toHaveBeenCalledWith("/projects/p1/settings");
  });

  it("会话入口区已撤：无会话行/查看全部按钮", async () => {
    const w = await mountOverview();
    expect(w.find(".conv-section").exists()).toBe(false);
    expect(w.find(".view-all").exists()).toBe(false);
  });

  it("成员负载：环图 + 排行行（名字/模型/token 主数字/消息小字），条按 token 归一", async () => {
    const w = await mountOverview();
    // 环图：2 段弧 + track，中心总量 9000+2700=11700 → 11.7K（K/M 惯用格式）
    expect(w.find(".donut").exists()).toBe(true);
    const segs = w.findAll(".donut-seg");
    expect(segs.length).toBe(2);
    expect(w.find(".donut-value").text()).toBe("11.7K");
    // 行：名字 + 模型 + token 主数字（9000→9K）+ 消息小字
    const rows = w.findAll(".share-row");
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("前端专家");
    expect(rows[0].text()).toContain("glm-5.2");
    expect(rows[0].find(".count-tokens").text()).toBe("9K");
    expect(rows[0].find(".count-msgs").text()).toBe("30 条");
    // 条宽按 token 归一：榜首 100%，次行 2700/9000 = 30%
    const bars = w.findAll(".share-bar");
    expect(bars[0].attributes("style")).toContain("width: 100%");
    expect(bars[1].attributes("style")).toContain("width: 30%");
    // title 挂在 label 格（行容器 display:contents 不渲染盒子）——带占比 9000/11700 = 77%
    expect(w.findAll(".share-label")[0].attributes("title")).toContain("77%");
  });

  it("token 全零（估算未回填）：环图隐藏，条回退消息口径", async () => {
    mockBackend([], {
      ...OVERVIEW_OUT,
      agent_shares: [
        { agent_id: "a1", messages: 30, tokens: 0 },
        { agent_id: "a2", messages: 12, tokens: 0 },
      ],
    });
    const w = await mountOverview();
    expect(w.find(".donut").exists()).toBe(false);
    const rows = w.findAll(".share-row");
    expect(rows[0].find(".count-tokens").text()).toBe("30");
    expect(rows[0].find(".count-msgs").exists()).toBe(false);
    expect(w.findAll(".share-bar")[1].attributes("style")).toContain("width: 40%");
  });

  it("成员负载 >5 截断为 Top5 + 「其他 N 位」聚合行（消息与 token 双聚合）", async () => {
    mockBackend([], {
      ...OVERVIEW_OUT,
      agent_shares: Array.from({ length: 8 }, (_, i) => ({
        agent_id: `a${i}`, messages: 10 - i, tokens: (10 - i) * 100,
      })),
    });
    const w = await mountOverview();
    const rows = w.findAll(".share-row");
    // Top5 + 其他 1 行
    expect(rows.length).toBe(6);
    expect(rows[5].text()).toContain("其他 3 位");
    // 聚合：消息 5+4+3=12，token 500+400+300=1200 → 1.2K
    expect(rows[5].find(".count-tokens").text()).toBe("1.2K");
    // 环图 6 段（含其他聚合段）
    expect(w.findAll(".donut-seg").length).toBe(6);
  });

  it("hover 联动：弧段 hover → 聚焦段变粗 + 其余段/行淡化 + 中心切为该成员值；行 hover 反向联动", async () => {
    const w = await mountOverview();
    // 默认态：中心总量 + 字距小标 TOKENS
    expect(w.find(".donut-label").text()).toBe("TOKENS");
    // hover 第 2 段（a2，2700 tokens → 2.7K）
    await w.findAll(".donut-seg")[1].trigger("mouseenter");
    const segs = w.findAll(".donut-seg");
    expect(segs[1].classes()).toContain("active"); // 聚焦段变粗
    expect(segs[0].classes()).toContain("dim");   // 其余段淡出
    const rows = w.findAll(".share-row");
    expect(rows[1].classes()).toContain("hovered");
    expect(rows[0].classes()).toContain("dim");    // 其余行同步淡出
    expect(w.find(".donut-value").text()).toBe("2.7K");
    expect(w.find(".donut-sub-name").text()).toContain("测试专家");
    // 移出环图恢复总量态
    await w.find(".donut").trigger("mouseleave");
    expect(w.find(".donut-value").text()).toBe("11.7K");
    expect(w.findAll(".share-row")[1].classes()).not.toContain("hovered");

    // 反向：hover 第 1 行 label → 对应段变粗、其余淡化、中心切换
    await w.findAll(".share-label")[0].trigger("mouseenter");
    const segs2 = w.findAll(".donut-seg");
    expect(segs2[0].classes()).toContain("active");
    expect(segs2[1].classes()).toContain("dim");
    expect(w.find(".donut-value").text()).toBe("9K");
    // 移出排行区清空
    await w.find(".share-rows").trigger("mouseleave");
    expect(w.find(".donut-value").text()).toBe("11.7K");
  });

  it("环图几何：段间视觉间隙恒定（round cap 延伸已计入 dash），无重叠", async () => {
    const w = await mountOverview();
    // 从 DOM 读 dash 几何：dasharray[0] = dash 长，dashoffset = -start
    const read = (i: number) => {
      const seg = w.findAll(".donut-seg")[i]!;
      return {
        dash: parseFloat(seg.attributes("stroke-dasharray")!.split(" ")[0]!),
        start: -parseFloat(seg.attributes("stroke-dashoffset")!),
      };
    };
    const CAP = 5.5; // round cap 每端延伸 = 线宽 11 / 2（与组件 STROKE 常量同步）
    const a = read(0);
    const b = read(1);
    // 段 0 视觉终点 = start + dash + CAP；段 1 视觉起点 = start - CAP
    const gap = b.start - CAP - (a.start + a.dash + CAP);
    expect(gap).toBeCloseTo(4, 5); // SEG_GAP 恒定，不随段长/占比变化
    // 段 0 视觉长 = 9000/11700 周长 - 间隙
    const C = 2 * Math.PI * 40;
    expect(a.dash + CAP * 2).toBeCloseTo((9000 / 11700) * C - 4, 5);
  });

  it("成员负载空态：无消息成员不出现", async () => {
    mockBackend([], { ...OVERVIEW_OUT, agent_shares: [] });
    const w = await mountOverview();
    expect(w.find(".share-row").exists()).toBe(false);
    expect(w.find(".share-card").text()).toContain("成员暂无消息");
  });
});
