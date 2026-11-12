// TaskLedger.test.ts — 任务面板 v2（批 P2-2 person-first）行为锁定：
// 分区排序（等你回应 > 执行中 > 失败受阻 > 已完成）/ 行点击跳轨迹 /
// interrupted 诚实归「失败受阻」区 / 发起者与执行者解析 / 空态引导。
// v1 旧锁（running 置顶等）由 taskBoard 纯函数测试覆盖同一语义。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import TaskLedger from "../TaskLedger.vue";
import { useChatStore } from "../../../stores/chat";
import { useAgentStore } from "../../../stores/agent";
import type { ProjectTask } from "../../../types";

const mockInvoke = vi.mocked(invoke);
const push = vi.fn();

vi.mock("vue-router", () => ({
  useRouter: () => ({ push }),
}));

function task(overrides: Partial<ProjectTask>): ProjectTask {
  return {
    conv_id: "c",
    title: "任务",
    executor_agent_id: "a1",
    initiator_agent_id: null,
    parent_conversation_id: null,
    started_at: "2026-08-18 10:00:00",
    updated_at: "2026-08-18 10:05:00",
    ended_at: "2026-08-18 10:05:00",
    termination: "stop",
    rounds: 2,
    ...overrides,
  };
}

function seedStores() {
  const chat = useChatStore();
  const agent = useAgentStore();
  agent.list = [
    {
      id: "a1", name: "执行专家", provider: "anthropic", model: "m", system_prompt: "",
      base_url: null, temperature: 0.7, max_tokens: 1, extra_params: {}, sort_order: 0,
      cache_prompt: false, has_api_key: true,
      created_at: "2026-08-18 00:00:00", updated_at: "2026-08-18 00:00:00",
    } as never,
  ];
  return { chat, agent };
}

function sectionsOf(w: ReturnType<typeof mount>) {
  return w.findAll(".board-section").map((s) => s.attributes("data-section"));
}

describe("TaskLedger 任务面板 v2（批 P2-2）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset().mockResolvedValue([] as never);
    push.mockReset();
  });

  it("分区排序：等你回应 > 执行中 > 失败受阻 > 已完成", () => {
    const { chat } = seedStores();
    // r1 在途（streaming 桶）；w1 在途且有未答 ask（waiting 最高优先）
    chat.ensureStream("r1").sending = true;
    chat.ensureStream("w1").sending = true;
    chat.pendingAskRequests = new Map([
      ["w1", { payload: { request_id: "q1", question: "用哪个版本？" } } as never],
    ]);

    const w = mount(TaskLedger, {
      props: {
        tasks: [
          task({ conv_id: "old-done", termination: "stop", updated_at: "2026-08-18 09:00:00" }),
          task({ conv_id: "r1", termination: null, ended_at: null, updated_at: "2026-08-18 08:00:00" }),
          task({ conv_id: "w1", termination: null, ended_at: null, updated_at: "2026-08-18 08:30:00" }),
          task({ conv_id: "f1", termination: "budget_exceeded", updated_at: "2026-08-18 11:00:00" }),
          task({ conv_id: "new-done", termination: "stop", updated_at: "2026-08-18 12:00:00" }),
        ],
      },
    });
    expect(sectionsOf(w)).toEqual(["waiting", "running", "failed", "done"]);
    // 等你回答的问题摘要在行内呈现
    const wSec = w.find('.board-section[data-section="waiting"]');
    expect(wSec.text()).toContain("执行专家");
    expect(wSec.text()).toContain("等你回答");
    // 头部摘要计数在场
    expect(w.find(".board-meta").text()).toContain("1 个等你回应");
  });

  it("行点击 → 落到该子会话轨迹（openConversationAtTrajectory）", async () => {
    seedStores();
    const chat = useChatStore();
    const spy = vi.spyOn(chat, "openConversationAtTrajectory");
    const w = mount(TaskLedger, {
      props: { tasks: [task({ conv_id: "child-1", parent_conversation_id: "parent" })] },
    });
    await w.find(".task-row").trigger("click");
    expect(spy).toHaveBeenCalledWith("child-1");
  });

  it("interrupted 诚实归「失败受阻」区（无 turn_ended 且非流式）", () => {
    seedStores();
    const w = mount(TaskLedger, {
      props: { tasks: [task({ conv_id: "c", termination: null, ended_at: null })] },
    });
    expect(sectionsOf(w)).toEqual(["failed"]);
    // 状态词走 StatusGlyph aria-label（可见态由 glyph 语系承载：叉/中性环）
    expect(w.find(".task-row [role='img']").attributes("aria-label")).toBe("中断");
  });

  it("发起者标注：agent 名解析 / null ≡ 用户发起（不显示发起标注）", () => {
    seedStores();
    const w = mount(TaskLedger, {
      props: {
        tasks: [
          task({ conv_id: "u", executor_agent_id: "a1", initiator_agent_id: null }),
          task({ conv_id: "g", executor_agent_id: "a1", initiator_agent_id: "a1" }),
        ],
      },
    });
    const rows = w.findAll(".task-row");
    expect(rows[0].text()).not.toContain("发起");
    expect(rows[1].text()).toContain("执行专家 发起");
  });

  it("空态引导文案", () => {
    seedStores();
    const w = mount(TaskLedger, { props: { tasks: [] } });
    expect(w.find(".board-empty").text()).toContain("暂无任务");
  });
});
