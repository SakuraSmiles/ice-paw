// ConfigProposalCard.tool-scopes.test.ts — 工具集范围提案的分派锁（2026-09-12 批）：
// ① update 提案带 tool_scopes → 批准走旋钮通道 setToolScopes（不进 update_agent
//    的出生证字段）；空数组 = 摘除语义原样透传；② create 提案带 tool_scopes →
//    先 create（默认全开）后 setToolScopes 收窄；③ 不带 tool_scopes 的提案零调用。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import type { ConfigProposalPayload } from "../../../types";

const respondMock = vi.fn();
const agentLoadMock = vi.fn();
const updateMock = vi.fn();
const createMock = vi.fn();
const setToolScopesMock = vi.fn();
const setEnabledToolsMock = vi.fn();
const setWordProfileMock = vi.fn();
const setYamlFieldMock = vi.fn();
const setSystemPromptMock = vi.fn();

vi.mock("../../../stores/chat", () => ({
  useChatStore: () => ({ respondToProposal: respondMock }),
}));
vi.mock("../../../stores/agent", () => ({
  useAgentStore: () => ({ load: agentLoadMock }),
}));
vi.mock("../../../api/bridge", () => ({
  bridge: {
    agents: {
      create: (...a: unknown[]) => createMock(...a),
      update: (...a: unknown[]) => updateMock(...a),
      setToolScopes: (...a: unknown[]) => setToolScopesMock(...a),
      setEnabledTools: (...a: unknown[]) => setEnabledToolsMock(...a),
      setWordProfile: (...a: unknown[]) => setWordProfileMock(...a),
      setYamlField: (...a: unknown[]) => setYamlFieldMock(...a),
      setSystemPrompt: (...a: unknown[]) => setSystemPromptMock(...a),
    },
  },
}));

function makeProposal(action: unknown): ConfigProposalPayload {
  return {
    request_id: "req-1",
    conversation_id: "c1",
    message_id: "m1",
    tool_use_id: "t1",
    sensitivity: "medium",
    action: action as ConfigProposalPayload["action"],
    summary: "测试提案",
  };
}

async function mountCard(proposal: ConfigProposalPayload) {
  const { default: ConfigProposalCard } = await import("../ConfigProposalCard.vue");
  const w = mount(ConfigProposalCard, { props: { proposal } });
  await flushPromises();
  return w;
}

async function clickApprove(w: Awaited<ReturnType<typeof mountCard>>) {
  await w.find(".btn-approve").trigger("click");
  await flushPromises();
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("ConfigProposalCard tool_scopes 分派", () => {
  it("update 提案带 tool_scopes → 旋钮通道 setToolScopes，不进出生证 update", async () => {
    const w = await mountCard(
      makeProposal({
        action: "update_agent",
        agent_id: "ag-1",
        tool_scopes: ["group:files", "server:ue5"],
      }),
    );
    await clickApprove(w);

    expect(setToolScopesMock).toHaveBeenCalledTimes(1);
    expect(setToolScopesMock).toHaveBeenCalledWith("ag-1", ["group:files", "server:ue5"]);
    // 出生证通道只收到 id 与全 undefined 字段（tool_scopes 不混进 update_agent）
    expect(updateMock).toHaveBeenCalledWith({
      id: "ag-1",
      name: undefined,
      provider: undefined,
      model: undefined,
      base_url: undefined,
      workspace_path: undefined,
    });
    expect(respondMock).toHaveBeenCalled();
  });

  it("update 提案 tool_scopes 空数组 = 摘除语义原样透传", async () => {
    const w = await mountCard(
      makeProposal({
        action: "update_agent",
        agent_id: "ag-1",
        tool_scopes: [],
      }),
    );
    await clickApprove(w);

    expect(setToolScopesMock).toHaveBeenCalledWith("ag-1", []);
  });

  it("create 提案带 tool_scopes → 先 create 后 setToolScopes 收窄", async () => {
    createMock.mockResolvedValue({});
    const w = await mountCard(
      makeProposal({
        action: "create_agent",
        id: "new-bot",
        name: "新助手",
        provider: "glm",
        model: "glm-5.3",
        api_key: "__SLOT__",
        tool_scopes: ["group:docx"],
      }),
    );
    await w.find('input[type="password"]').setValue("sk-user-key");
    await clickApprove(w);

    expect(createMock).toHaveBeenCalledTimes(1);
    expect(setToolScopesMock).toHaveBeenCalledTimes(1);
    expect(setToolScopesMock).toHaveBeenCalledWith("new-bot", ["group:docx"]);
    // create 先于 scopes 收窄（收窄依赖 agent 已存在）
    expect(createMock.mock.invocationCallOrder[0]).toBeLessThan(
      setToolScopesMock.mock.invocationCallOrder[0],
    );
  });

  it("不带 tool_scopes 的提案零调用（存量提案行为不变）", async () => {
    const w = await mountCard(
      makeProposal({
        action: "update_agent",
        agent_id: "ag-1",
        temperature: 0.3,
      }),
    );
    await clickApprove(w);

    expect(setToolScopesMock).not.toHaveBeenCalled();
    expect(setEnabledToolsMock).not.toHaveBeenCalled();
    // temperature 走 yaml 通道照旧
    expect(setYamlFieldMock).toHaveBeenCalledWith("ag-1", "temperature", 0.3);
  });
});
