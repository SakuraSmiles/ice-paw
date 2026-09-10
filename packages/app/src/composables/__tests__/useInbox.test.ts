// useInbox 单测（MA-3 收件箱前端状态层）。
// 锁定语义：
// - boot counts 批量载入 / 事件增量算术（来件 +1、settled 抵扣删零）
// - 其它 kind 零干扰（session:event-appended 是全事件总线，不是专属通道）
// - 失焦通知三重 gating：失焦才查 + 权威列表按政策分流（hold 待批准 / accept
//   知会；refuse 兜底不发）+ 恰一次簿记
// - refreshInboxCount 权威回正（badge 是气味，popover 是真相）
// - cleanup 拆监听
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { listen } from "@tauri-apps/api/event";
import { bridge } from "../../api/bridge";
import type { MockInstance } from "vitest";
import { useInbox, initInbox, refreshInboxCount } from "../useInbox";
import type { InboxView } from "../../types";

vi.mock("../../api/bridge", () => ({
  bridge: {
    inbox: {
      counts: vi.fn().mockResolvedValue([]),
      list: vi.fn().mockResolvedValue({ policy: "hold", items: [] }),
      respond: vi.fn(),
      setPolicy: vi.fn(),
    },
    chat: { notifyApproval: vi.fn().mockResolvedValue(undefined) },
  },
}));

const mockCounts = vi.mocked(bridge.inbox.counts);
const mockList = vi.mocked(bridge.inbox.list);
const mockNotify = vi.mocked(bridge.chat.notifyApproval);

/** 捕获 initInbox 注册的事件 handler（setup.ts 的 listen mock 被 override） */
type ListenFn = typeof listen;
const handlers = new Map<string, (e: { payload: unknown }) => void>();
vi.mocked(listen).mockImplementation(
  async (event: Parameters<ListenFn>[0], handler: Parameters<ListenFn>[1]) => {
    handlers.set(String(event), handler as unknown as (e: { payload: unknown }) => void);
    return () => {
      handlers.delete(String(event));
    };
  },
);

function itemView(policy: string, messageIds: string[]): InboxView {
  return {
    policy,
    items: messageIds.map((id, i) => ({
      message_id: id,
      source_conversation_id: "c-src",
      source_conversation_title: "主控",
      source_agent_id: "a1",
      source_agent_name: "甲",
      content: `第 ${i + 1} 条来件内容`,
      expect_reply: false,
      delivered_at_unix: 1700000000,
    })),
  };
}

function arrive(cid: string, kind: string) {
  handlers.get("session:event-appended")!({ payload: { conversation_id: cid, kind } });
}

/** flush fire-and-forget 微任务链（void 调用 → await list → 通知落定） */
async function flush() {
  await new Promise((r) => setTimeout(r, 0));
}

describe("useInbox（MA-3 收件箱计数）", () => {
  let hasFocusSpy: MockInstance;
  let cleanup: () => void;
  const { pendingCounts, pendingOf } = useInbox();

  beforeEach(async () => {
    handlers.clear();
    pendingCounts.clear();
    mockCounts.mockReset().mockResolvedValue([]);
    mockList.mockReset().mockResolvedValue(itemView("hold", []));
    mockNotify.mockClear().mockResolvedValue(undefined);
    hasFocusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    cleanup = await initInbox();
    await flush();
  });

  afterEach(() => {
    cleanup();
    hasFocusSpy.mockRestore();
  });

  it("boot：counts 批量载入；无记录会话 pendingOf=0", async () => {
    mockCounts.mockResolvedValue([["c1", 2], ["c2", 1]]);
    const c = await initInbox();
    await flush();
    expect(pendingOf("c1")).toBe(2);
    expect(pendingOf("c2")).toBe(1);
    expect(pendingOf("c-unknown")).toBe(0);
    c();
  });

  it("事件增量：来件 +1 累加；settled 抵扣、扣到零删键", () => {
    arrive("c1", "cross_session_message");
    expect(pendingOf("c1")).toBe(1);
    arrive("c1", "cross_session_message");
    expect(pendingOf("c1")).toBe(2);

    arrive("c1", "cross_session_message_settled");
    expect(pendingOf("c1")).toBe(1);
    arrive("c1", "cross_session_message_settled");
    expect(pendingOf("c1")).toBe(0);
    expect(pendingCounts.has("c1")).toBe(false); // 删键（侧栏 badge v-if 归零）
  });

  it("其它 kind 零干扰（全事件总线上各走各的）", () => {
    arrive("c1", "user_message");
    arrive("c1", "tool_execution");
    arrive("c1", "model_switch");
    expect(pendingOf("c1")).toBe(0);
  });

  it("失焦 + hold：权威列表确认后逐条通知，恰一次；重复到达不重发", async () => {
    hasFocusSpy.mockReturnValue(false);
    mockList.mockResolvedValue(itemView("hold", ["m1", "m2"]));

    arrive("c1", "cross_session_message");
    await flush();
    expect(mockList).toHaveBeenCalledWith("c1");
    expect(mockNotify).toHaveBeenCalledTimes(2); // 列表两条各一条
    expect(mockNotify).toHaveBeenLastCalledWith({
      title: "IcePaw · 跨会话消息待批准",
      body: '来自「主控」的 agent 甲：第 2 条来件内容',
      request_id: undefined, // 纯提醒：不进 toast 按钮协议
    });

    // 同一会话又来一件（m1 仍在队中）：只通知新到的
    mockList.mockResolvedValue(itemView("hold", ["m1", "m2", "m3"]));
    arrive("c1", "cross_session_message");
    await flush();
    expect(mockNotify).toHaveBeenCalledTimes(3);
  });

  it("失焦 + accept：知会性通知（2026-09-10 默认改 accept 后通知不再只服务 hold）", async () => {
    hasFocusSpy.mockReturnValue(false);
    mockList.mockResolvedValue(itemView("accept", ["m1"]));
    arrive("c1", "cross_session_message");
    await flush();
    expect(mockList).toHaveBeenCalledWith("c1"); // 权威确认照走（policy 是后端真相）
    expect(mockNotify).toHaveBeenCalledTimes(1);
    expect(mockNotify).toHaveBeenCalledWith({
      title: "IcePaw · 收到跨会话消息", // 非「待批准」——自动消费，纯知会
      body: '来自「主控」的 agent 甲：第 1 条来件内容',
      request_id: undefined,
    });
  });

  it("失焦 + refuse：不通知（收不到来件的政策值，兜底防御）", async () => {
    hasFocusSpy.mockReturnValue(false);
    mockList.mockResolvedValue(itemView("refuse", ["m1"]));
    arrive("c1", "cross_session_message");
    await flush();
    expect(mockList).toHaveBeenCalledWith("c1");
    expect(mockNotify).not.toHaveBeenCalled();
  });

  it("聚焦：不查列表不发通知（应用内 badge 已可见）", async () => {
    hasFocusSpy.mockReturnValue(true);
    arrive("c1", "cross_session_message");
    await flush();
    expect(mockList).not.toHaveBeenCalled();
    expect(mockNotify).not.toHaveBeenCalled();
  });

  it("refreshInboxCount：权威数回正漂移；空队列删键；失败保持现状", async () => {
    arrive("c1", "cross_session_message");
    arrive("c1", "cross_session_message");
    arrive("c1", "cross_session_message"); // 本地 3

    mockList.mockResolvedValue(itemView("hold", ["m1"])); // 真相 1（丢帧漂移）
    await refreshInboxCount("c1");
    expect(pendingOf("c1")).toBe(1);

    mockList.mockResolvedValue(itemView("hold", [])); // 清空
    await refreshInboxCount("c1");
    expect(pendingOf("c1")).toBe(0);

    arrive("c1", "cross_session_message");
    mockList.mockRejectedValue(new Error("db busy"));
    await refreshInboxCount("c1"); // 失败不回零、不清计数
    expect(pendingOf("c1")).toBe(1);
  });

  it("cleanup：事件 handler 拆卸（后续 bus 消息不再动计数）", () => {
    cleanup();
    expect(handlers.has("session:event-appended")).toBe(false);
  });
});
