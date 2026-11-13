import { describe, expect, it } from "vitest";
import { scheduledTaskNotification, scheduledTaskReason, scheduledTaskStatus } from "../scheduledTaskStatus";

describe("定时任务终态展示契约", () => {
  it.each(["error", "stopped", "missed", "unexpected", "toString"])(
    "%s 即使带有正常结果形状的输出，也不会显示为成功或发送完成通知", (status) => {
      const run = { status, task_name: "日报", summary: "报告已生成", error: "预算已耗尽" };
      expect(scheduledTaskStatus(status).tone).not.toBe("success");
      expect(scheduledTaskNotification(run)?.title).not.toContain("已完成");
      expect(scheduledTaskNotification(run)?.body).toBe("预算已耗尽");
    },
  );

  it("旧失败记录没有 error 时给出可操作提示，不以部分输出冒充原因", () => {
    const run = { status: "error", task_name: "日报", summary: "已完成前半段" };
    expect(scheduledTaskReason(run)).toContain("查看执行记录");
    expect(scheduledTaskNotification(run)?.body).not.toContain("前半段");
  });

  it("正常完成展示结果首行，运行中不产生终态通知", () => {
    expect(scheduledTaskNotification({ status: "done", task_name: "日报", summary: "结果首行\n后续内容" }))
      .toEqual({ title: "定时任务已完成：日报", body: "结果首行" });
    expect(scheduledTaskNotification({ status: "running", task_name: "日报" })).toBeNull();
  });
});
