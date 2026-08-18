// useProjectTasks.test.ts — 台账数据源 live 更新过滤锁定：
// turn_ended 只认任务集内会话（流式 chunk 噪声被 kind 过滤零动作）、
// delegation-started 无项目字段宁多刷、去抖 300ms 合并连发。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useProjectTasks } from "../useProjectTasks";
import type { ProjectTask } from "../../types";

const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);

function captureHandlers() {
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  mockListen.mockImplementation(async (event, handler) => {
    handlers.set(event, handler as (event: { payload: unknown }) => void);
    return () => { handlers.delete(event, ); };
  });
  return handlers;
}

function task(conv_id: string): ProjectTask {
  return {
    conv_id, title: conv_id, executor_agent_id: "a1", initiator_agent_id: null,
    parent_conversation_id: null, started_at: "2026-08-18 10:00:00",
    updated_at: "2026-08-18 10:05:00", ended_at: "2026-08-18 10:05:00",
    termination: null, rounds: null,
  };
}

/** 宿主组件（composable 的 onMounted/onBeforeUnmount 需组件上下文） */
function mountHost(pid = () => "p1") {
  let api!: ReturnType<typeof useProjectTasks>;
  const Host = defineComponent({
    setup() {
      api = useProjectTasks(pid);
      return () => h("div");
    },
  });
  mount(Host);
  return api;
}

describe("useProjectTasks live 更新", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset().mockResolvedValue([task("child-1")] as never);
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("挂载即拉一次台账", async () => {
    const handlers = captureHandlers();
    mountHost();
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("list_project_tasks", { projectId: "p1" });
    expect(handlers.size).toBe(2);
  });

  it("turn_ended 命中任务集 → 去抖后 refresh；非任务集/其他 kind 零动作", async () => {
    const handlers = captureHandlers();
    mountHost();
    await flushPromises();
    mockInvoke.mockClear();

    // 噪声：非 turn_ended（流式每轮大量 chunk 类事件）
    handlers.get("session:event-appended")!({ payload: { kind: "assistant_message", conversation_id: "child-1" } });
    // 噪声：turn_ended 但会话不在任务集（本会话每轮都发 turn_ended）
    handlers.get("session:event-appended")!({ payload: { kind: "turn_ended", conversation_id: "别的会话" } });
    await vi.advanceTimersByTimeAsync(400);
    expect(mockInvoke).not.toHaveBeenCalled();

    // 命中：任务集内的 turn_ended
    handlers.get("session:event-appended")!({ payload: { kind: "turn_ended", conversation_id: "child-1" } });
    await vi.advanceTimersByTimeAsync(400);
    expect(mockInvoke).toHaveBeenCalledWith("list_project_tasks", { projectId: "p1" });
  });

  it("连发 turn_ended 去抖合并为一次拉取", async () => {
    const handlers = captureHandlers();
    mountHost();
    await flushPromises();
    mockInvoke.mockClear();

    for (let i = 0; i < 3; i++) {
      handlers.get("session:event-appended")!({ payload: { kind: "turn_ended", conversation_id: "child-1" } });
      await vi.advanceTimersByTimeAsync(100);
    }
    await vi.advanceTimersByTimeAsync(400);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });

  it("delegation-started 任何委派都 refresh（payload 无项目字段，宁多刷不漏刷）", async () => {
    const handlers = captureHandlers();
    mountHost();
    await flushPromises();
    mockInvoke.mockClear();

    handlers.get("chat:delegation-started")!({ payload: { conversation_id: "他项目父会话" } });
    await vi.advanceTimersByTimeAsync(400);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });
});
