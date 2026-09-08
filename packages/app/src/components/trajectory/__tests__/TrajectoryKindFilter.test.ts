// TrajectoryKindFilter.test.ts — 「类型」多选下拉组件回归：
// 受控渲染（勾选态 = !hidden）/ toggle emit 增删键 / 恢复默认 emit DEFAULT_HIDDEN /
// 开合交互（点外部/Esc 关闭——capture 阶段监听范式）/ 计数徽标与触发药丸亮态。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import TrajectoryKindFilter from "../TrajectoryKindFilter.vue";
import { DEFAULT_HIDDEN, FILTER_GROUPS, type FilterKey } from "../../../composables/useTrajectory";

function mountFilter(props: { hidden?: FilterKey[]; counts?: Partial<Record<FilterKey, number>> } = {}) {
  return mount(TrajectoryKindFilter, {
    props: { hidden: props.hidden ?? [...DEFAULT_HIDDEN], ...(props.counts ? { counts: props.counts } : {}) },
    attachTo: document.body, // document 级 capture 监听需要真实挂载
  });
}

function rowOf(w: ReturnType<typeof mountFilter>, label: string) {
  const row = w.findAll(".tkf-row").find((r) => r.find(".tkf-label").text() === label);
  expect(row, `复选行「${label}」应存在`).toBeTruthy();
  return row!;
}

describe("TrajectoryKindFilter（类型多选下拉）", () => {
  it("打开后渲染 4 组全部 11 键（与 FILTER_GROUPS 同源）；默认隐藏集无恢复按钮", async () => {
    const w = mountFilter();
    expect(w.find(".tkf-pop").exists()).toBe(false); // 默认关
    await w.find(".tkf-btn").trigger("click");
    expect(w.find(".tkf-pop").exists()).toBe(true);
    expect(w.findAll(".tkf-group").map((g) => g.text())).toEqual(FILTER_GROUPS.map((g) => g.label));
    expect(w.findAll(".tkf-row")).toHaveLength(11);
    expect(w.find(".tkf-reset").exists()).toBe(false); // isDefault 态不显示恢复入口
    w.unmount();
  });

  it("受控勾选态：hidden 中的键不勾、其余勾；toggle emit 增删键", async () => {
    const w = mountFilter({ hidden: ["tool", "modal_adapted"] });
    await w.find(".tkf-btn").trigger("click");

    const checked = (label: string) => (rowOf(w, label).find("input[type=checkbox]").element as HTMLInputElement).checked;
    expect(checked("工具调用")).toBe(false); // hidden → 不勾
    expect(checked("视觉适配")).toBe(false);
    expect(checked("用户消息")).toBe(true); // 可见 → 勾

    // 勾回「工具调用」→ emit 的 hidden 里删掉它
    await rowOf(w, "工具调用").find("input[type=checkbox]").setValue(true);
    let emitted = w.emitted("update:hidden") as [FilterKey[]][];
    expect(emitted[emitted.length - 1][0].sort()).toEqual(["modal_adapted"]);

    // 取消勾「用户消息」→ emit 的 hidden 里加上它。组件纯受控（toggle 从 props.hidden
    // 出发，emit 不自改）——此处 props 仍是初值 ["tool","modal_adapted"]，加 user 得三键
    await rowOf(w, "用户消息").find("input[type=checkbox]").setValue(false);
    emitted = w.emitted("update:hidden")!;
    expect(emitted[emitted.length - 1][0].sort()).toEqual(["modal_adapted", "tool", "user"]);
    w.unmount();
  });

  it("非默认态显示恢复按钮：emit DEFAULT_HIDDEN；触发药丸亮态同步", async () => {
    const w = mountFilter({ hidden: ["tool"] });
    expect(w.find(".tkf-btn").classes()).toContain("active"); // ≠ 默认集 → 亮
    await w.find(".tkf-btn").trigger("click");
    expect(w.find(".tkf-reset").exists()).toBe(true);
    await w.find(".tkf-reset").trigger("click");
    const resets = w.emitted("update:hidden") as [FilterKey[]][];
    expect(resets[resets.length - 1][0]).toEqual([...DEFAULT_HIDDEN]);
    w.unmount();
  });

  it("计数徽标：counts 提供且非 0 的键渲染 mono 数字，其余不渲染", async () => {
    const w = mountFilter({ counts: { user: 12, tool: 0 } });
    await w.find(".tkf-btn").trigger("click");
    expect(rowOf(w, "用户消息").find(".tkf-count").text()).toBe("12");
    expect(rowOf(w, "工具调用").find(".tkf-count").exists()).toBe(false); // 0 不显示
    w.unmount();
  });

  it("点外部与 Esc 关闭浮层（capture 阶段监听范式）", async () => {
    const w = mountFilter();
    await w.find(".tkf-btn").trigger("click");
    expect(w.find(".tkf-pop").exists()).toBe(true);

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await w.vm.$nextTick();
    expect(w.find(".tkf-pop").exists()).toBe(false);

    await w.find(".tkf-btn").trigger("click"); // 重开
    document.body.click(); // 组件外部点击（capture 拦截）
    await w.vm.$nextTick();
    expect(w.find(".tkf-pop").exists()).toBe(false);
    w.unmount();
  });
});
