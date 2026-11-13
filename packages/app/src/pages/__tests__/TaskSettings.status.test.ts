import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import TaskSettings from "../settings/TaskSettings.vue";
import type { ScheduledTaskView, TaskRun } from "../../types";

const mocks = vi.hoisted(() => ({ list: vi.fn(), runs: vi.fn(), preview: vi.fn() }));
vi.mock("../../api/bridge", () => ({ bridge: { tasks: mocks } }));
vi.mock("vue-router", () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock("../../stores/agent", () => ({ useAgentStore: () => ({ list: [], visibleAgents: [] }) }));
vi.mock("../../stores/chat", () => ({ useChatStore: () => ({ conversations: [] }) }));
vi.mock("../../stores/project", () => ({ useProjectStore: () => ({}), TASK_SCOPE: "__scheduled_tasks__" }));

function run(status: string, error: string | null = null): TaskRun {
  return {
    id: "r-1", task_id: "t-1", conv_id: "c-1", status, error, summary: "已分析两个文件\n第三个尚未处理",
    started_at: "2026-11-13 00:00:00", finished_at: "2026-11-13 00:01:00",
  };
}
function task(last_run: TaskRun): ScheduledTaskView {
  return {
    id: "t-1", name: "检查文档", agent_id: "a-1", schedule_kind: "once", schedule_data: "{}",
    prompt: "检查文档", target_conv_id: "c-1", miss_policy: "skip", deliver_to_conv_id: null,
    preauth: "none", enabled: 1, next_run: null, last_run_at: last_run.started_at,
    created_at: last_run.started_at, updated_at: last_run.started_at, last_run,
  };
}
let wrapper: VueWrapper | undefined;
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  mocks.preview.mockResolvedValue([]);
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  vi.clearAllTimers();
  vi.useRealTimers();
});

describe("定时任务执行记录保留原因与部分输出", () => {
  it.each([
    ["error", "未完成", "输出长度达到上限"],
    ["stopped", "已停止", "任务已由你停止"],
  ])("%s 显示准确终态和原因，保留部分输出", async (status, label, error) => {
    const record = run(status, error);
    mocks.list.mockResolvedValue([task(record)]);
    mocks.runs.mockResolvedValue([record]);
    wrapper = mount(TaskSettings);
    await flushPromises();
    const card = wrapper.findAll(".profile-card")[1];
    expect(card.find(".health-chip").text()).toContain(label);
    expect(card.find(".next-run").text()).toBe("无后续计划");
    await card.trigger("click");
    await flushPromises();
    expect(wrapper.find(".run-chip").text()).toBe(label);
    expect(wrapper.find(".run-reason").text()).toBe(error);
    expect(wrapper.find(".run-output").text()).toContain("部分输出：已分析两个文件");
    expect(wrapper.find(".run-output").text()).toContain("第三个尚未处理");
  });

  it("正常完成保留完整结果且不标成部分输出", async () => {
    const record = run("done");
    mocks.list.mockResolvedValue([task(record)]);
    mocks.runs.mockResolvedValue([record]);
    wrapper = mount(TaskSettings);
    await flushPromises();
    await wrapper.findAll(".profile-card")[1].trigger("click");
    await flushPromises();
    expect(wrapper.find(".run-chip").text()).toBe("已完成");
    expect(wrapper.find(".run-reason").exists()).toBe(false);
    expect(wrapper.find(".run-output").text()).not.toContain("部分输出");
    expect(wrapper.find(".run-output").text()).toContain("已分析两个文件");
  });
});
