// Router 化批 P1-1 回归锁：ephemeral/router 永不进用户可见面
// 防护网先于 spawn_worker 落地（0.9.11 教训：新增数据形态先锁消费判定）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { useAgentStore } from "../agent";
import type { Agent } from "../../types";
import { bridge } from "../../api/bridge";

vi.mock("../../api/bridge", () => ({
  bridge: {
    agents: {
      list: vi.fn(),
      // store 外的其他成员按需补——本文件只测 load/list 链路
    },
  },
}));

function agentOf(id: string, kind?: string): Agent {
  return {
    id,
    name: `agent-${id}`,
    provider: "openai",
    model: "m",
    system_prompt: "",
    temperature: 0.7,
    max_tokens: 1024,
    extra_params: {},
    sort_order: 0,
    created_at: "2026-01-01 00:00:00",
    updated_at: "2026-01-01 00:00:00",
    has_api_key: true,
    ...(kind ? { agent_kind: kind } : {}),
  } as Agent;
}

describe("agent store visibleAgents（批 P1-1 收口）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("visibleAgents 过滤 ephemeral 与 router，undefined 视为 standard", async () => {
    vi.mocked(bridge.agents.list).mockResolvedValue([
      agentOf("std-1"),
      agentOf("w-1", "ephemeral"),
      agentOf("r-1", "router"),
      agentOf("std-2", "standard"),
    ]);
    const store = useAgentStore();
    await store.load(true);
    expect(store.list.length).toBe(4);
    expect(store.visibleAgents.map((a) => a.id)).toEqual(["std-1", "std-2"]);
  });

  it("firstAgent 永不落在 ephemeral/router 上（默认位防混入）", async () => {
    // 首行就是 ephemeral（sort_order 恰好靠前）——默认位仍须取到 standard
    vi.mocked(bridge.agents.list).mockResolvedValue([
      agentOf("w-1", "ephemeral"),
      agentOf("std-1"),
    ]);
    const store = useAgentStore();
    await store.load(true);
    expect(store.firstAgent?.id).toBe("std-1");
  });

  it("getById 保留全量解析（委派卡/台账要能显示 worker 名）", async () => {
    vi.mocked(bridge.agents.list).mockResolvedValue([
      agentOf("std-1"),
      agentOf("w-1", "ephemeral"),
    ]);
    const store = useAgentStore();
    await store.load(true);
    expect(store.getById("w-1")?.name).toBe("agent-w-1");
  });
});
