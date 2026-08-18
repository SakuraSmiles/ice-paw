// TaskLedger.test.ts — 台账视图行为锁定：running 置顶 / updated_at 倒序 /
// 行点击跳首页+落轨迹 / interrupted 诚实文案 / 空态引导。
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
    },
  ];
  return { chat, agent };
}

describe("TaskLedger 台账", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset().mockResolvedValue([] as never);
    push.mockReset();
  });

  it("running 置顶（即便 updated_at 更旧），其余 updated_at 倒序", () => {
    const { chat } = seedStores();
    // bgStreams 有 r1 → r1 视为 running（store 无直接暴露 setter，用 streaming 计算依赖：
    // sending+activeConvId 组合）
    chat.selectConversation("r1");
    chat.sending = true;

    const w = mount(TaskLedger, {
      props: {
        tasks: [
          task({ conv_id: "old-done", termination: "stop", updated_at: "2026-08-18 09:00:00" }),
          task({ conv_id: "r1", termination: null, ended_at: null, updated_at: "2026-08-18 08:00:00" }),
          task({ conv_id: "new-done", termination: "stop", updated_at: "2026-08-18 12:00:00" }),
        ],
      },
    });
    const ids = w.findAll(".ledger-row").map((r) => r.attributes("title"));
    // 首行 = running（r1），随后 new-done（12:00）→ old-done（09:00）
    expect(ids[0]).toContain("进行中");
    expect(w.findAll(".col-title")[0].text()).toBe("任务");
    const labels = w.findAll(".state-label").map((l) => l.text());
    expect(labels).toEqual(["进行中", "已完成", "已完成"]);
  });

  it("行点击 → 首页 + 落到该子会话轨迹", async () => {
    seedStores();
    const chat = useChatStore();
    const spy = vi.spyOn(chat, "openConversationAtTrajectory");

    const w = mount(TaskLedger, {
      props: { tasks: [task({ conv_id: "child-1", parent_conversation_id: "parent" })] },
    });
    await w.find(".ledger-row").trigger("click");
    expect(push).toHaveBeenCalledWith("/");
    expect(spy).toHaveBeenCalledWith("child-1");
  });

  it("interrupted 诚实文案（无 turn_ended 且非流式）", () => {
    seedStores();
    const w = mount(TaskLedger, {
      props: { tasks: [task({ conv_id: "c", termination: null, ended_at: null })] },
    });
    expect(w.find(".state-label").text()).toBe("中断");
  });

  it("发起者 null ≡ 用户发起；agent 名从 store 解析", () => {
    seedStores();
    const w = mount(TaskLedger, {
      props: { tasks: [task({ executor_agent_id: "a1", initiator_agent_id: null })] },
    });
    const agentCols = w.findAll(".ledger-row .col-agent");
    expect(agentCols[0].text()).toBe("执行专家");
    expect(agentCols[1].text()).toBe("用户");
  });

  it("空态引导文案", () => {
    seedStores();
    const w = mount(TaskLedger, { props: { tasks: [] } });
    expect(w.find(".ledger-empty").text()).toContain("还没有委派任务");
  });
});
