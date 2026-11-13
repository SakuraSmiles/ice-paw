import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import TaskQuickEntry from "../TaskQuickEntry.vue";
import type { RecentTaskRun } from "../../../types";

const mocks = vi.hoisted(() => ({ list: vi.fn(), recentRuns: vi.fn(), notify: vi.fn() }));
vi.mock("../../../api/bridge", () => ({ bridge: { tasks: mocks } }));
vi.mock("vue-router", () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: async () => true,
  requestPermission: async () => "granted",
  sendNotification: mocks.notify,
}));

function run(status: string, error: string | null = null): RecentTaskRun {
  return {
    run_id: "r-1", task_id: "t-1", task_name: "日报", status, error, summary: "已经生成的报告片段",
    started_at: "2026-11-13 00:00:00", finished_at: "2026-11-13 00:01:00",
  };
}
let wrapper: VueWrapper | undefined;
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  vi.clearAllMocks();
  vi.spyOn(document, "hidden", "get").mockReturnValue(true);
  mocks.list.mockResolvedValue([]);
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("定时任务侧栏和系统通知终态", () => {
  it.each([
    ["error", "未完成", "预算已耗尽", "danger"],
    ["stopped", "已停止", "任务已由你停止", "neutral"],
    ["missed", "已错过", "错过了计划执行时间", "neutral"],
  ])("%s 同时正确标注侧栏、原因与系统通知", async (status, label, error, tone) => {
    mocks.recentRuns.mockResolvedValue([run(status, error)]);
    wrapper = mount(TaskQuickEntry);
    await flushPromises();
    expect(wrapper.find(".entry-line .tag").text()).toBe(label);
    expect(wrapper.find(".entry-line .tag").classes()).toContain(`tag--${tone}`);
    expect(wrapper.find(".pop-reason").text()).toBe(error);
    expect(mocks.notify).toHaveBeenCalledWith({ title: `定时任务${label}：日报`, body: error });
    expect(mocks.notify.mock.calls[0][0].body).not.toContain("报告片段");
  });

  it("运行中转为停止只通知一次，后续轮询不重复发送", async () => {
    mocks.recentRuns.mockResolvedValue([run("running")]);
    wrapper = mount(TaskQuickEntry);
    await flushPromises();
    expect(mocks.notify).not.toHaveBeenCalled();
    mocks.recentRuns.mockResolvedValue([run("stopped", "用户停止了任务")]);
    await vi.advanceTimersByTimeAsync(30_000);
    await flushPromises();
    expect(mocks.notify).toHaveBeenCalledOnce();
    expect(mocks.notify.mock.calls[0][0].title).toBe("定时任务已停止：日报");
    await vi.advanceTimersByTimeAsync(30_000);
    await flushPromises();
    expect(mocks.notify).toHaveBeenCalledOnce();
  });

  it("done 才发送正常结果，前台仍不发送通知", async () => {
    mocks.recentRuns.mockResolvedValue([run("done")]);
    wrapper = mount(TaskQuickEntry);
    await flushPromises();
    expect(mocks.notify).toHaveBeenCalledWith({ title: "定时任务已完成：日报", body: "已经生成的报告片段" });
    wrapper.unmount();
    mocks.notify.mockClear();
    vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    wrapper = mount(TaskQuickEntry);
    await flushPromises();
    expect(mocks.notify).not.toHaveBeenCalled();
  });
});
