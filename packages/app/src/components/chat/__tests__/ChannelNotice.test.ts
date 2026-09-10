// ChannelNotice.test.ts — 频道行为事件通知条三 kind 判别锁定：
// channel_mention（正常点名 / 护栏拦截词表 / from 缺省 = 用户）、
// channel_election（started / vote 两态 / result 两态）、
// channel_coordinator（四 action 文案 + tone 二分）。
// 名字解析走 agent store，查无回退「已退出成员」（成员可能已删的诚实边界）。
import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import ChannelNotice from "../ChannelNotice.vue";
import { useAgentStore } from "../../../stores/agent";
import type { SessionEvent } from "../../../types";

function agent(id: string, name: string) {
  return {
    id, name, provider: "zhipu", model: "glm-5.3", system_prompt: "", base_url: null,
    temperature: 0.7, max_tokens: 4096, extra_params: {}, sort_order: 0,
    cache_prompt: true, has_api_key: true, created_at: "", updated_at: "",
  };
}

let n = 0;
function ev(kind: SessionEvent["kind"], payload: unknown): SessionEvent {
  n += 1;
  return {
    id: n, session_id: "s1", seq: n, kind, actor: "user",
    turn_id: null, message_id: null,
    payload: payload as never,
    created_at: "2026-09-10T10:00:00Z",
  };
}

function mountNotice(event: SessionEvent) {
  return mount(ChannelNotice, { props: { event } });
}

describe("ChannelNotice（频道事件通知条）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    const agents = useAgentStore();
    agents.list = [agent("ag1", "写手"), agent("ag2", "审校")];
  });

  it("channel_mention 正常跳：from 有值 = 成员接力点名；tone=info", () => {
    const w = mountNotice(ev("channel_mention", { from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 1, chain_remaining: 2, blocked_reason: null }));
    expect(w.find(".channel-notice-text").text()).toBe("写手 点名 审校 接力");
    expect(w.find(".channel-notice").classes()).toContain("info");
  });

  it("channel_mention from 缺省 = 用户点名", () => {
    const w = mountNotice(ev("channel_mention", { from_agent_id: null, to_agent_id: "ag1", hop_index: 1, chain_remaining: 0 }));
    expect(w.find(".channel-notice-text").text()).toBe("用户 点名 写手 接力");
  });

  it("channel_mention 拦截态：blocked_reason 中文词表 + tone=muted（user_preempted）", () => {
    const w = mountNotice(ev("channel_mention", { from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 0, chain_remaining: 0, blocked_reason: "user_preempted" }));
    expect(w.find(".channel-notice-text").text()).toBe("写手 @ 审校：用户插话，接力取消");
    expect(w.find(".channel-notice").classes()).toContain("muted");
  });

  it("channel_mention 拦截词表其余值（结构锁：六值全有中文文案）", () => {
    const cases: [string, string][] = [
      ["pair_repeat", "乒乓互 @ 拦截"],
      ["chain_limit", "接力链达上限"],
      ["frequency", "频率闸拦截"],
      ["ambiguous_name", "成员重名，点名歧义"],
      ["coordinator_failed", "统筹者故障，降级处理"],
    ];
    for (const [slug, label] of cases) {
      const w = mountNotice(ev("channel_mention", { from_agent_id: null, to_agent_id: "ag1", blocked_reason: slug }));
      expect(w.find(".channel-notice-text").text()).toBe(`点名 写手：${label}`);
    }
  });

  it("channel_election 三相：started / vote 投票 / result 当选（tone=info）", () => {
    const started = mountNotice(ev("channel_election", { phase: "started" }));
    expect(started.find(".channel-notice-text").text()).toBe("发起了统筹者自选举");
    expect(started.find(".channel-notice").classes()).toContain("muted");

    const vote = mountNotice(ev("channel_election", { phase: "vote", vote: { voter_agent_id: "ag1", candidate_agent_id: "ag2" } }));
    expect(vote.find(".channel-notice-text").text()).toBe("写手 投给 审校");

    const abstain = mountNotice(ev("channel_election", { phase: "vote", vote: { voter_agent_id: "ag2", candidate_agent_id: null } }));
    expect(abstain.find(".channel-notice-text").text()).toBe("审校 弃权");

    const result = mountNotice(ev("channel_election", { phase: "result", result: { tally: [{ agent_id: "ag2", votes: 1 }], winner_agent_id: "ag2" } }));
    expect(result.find(".channel-notice-text").text()).toBe("审校 当选统筹者");
    expect(result.find(".channel-notice").classes()).toContain("info");

    const noWinner = mountNotice(ev("channel_election", { phase: "result", result: { tally: [], winner_agent_id: null } }));
    expect(noWinner.find(".channel-notice-text").text()).toBe("全员弃权，统筹位空缺");
  });

  it("channel_coordinator 四 action：appointed/elected 走 info，removed/failed-over 走 muted", () => {
    const appointed = mountNotice(ev("channel_coordinator", { action: "appointed", agent_id: "ag1" }));
    expect(appointed.find(".channel-notice-text").text()).toBe("写手 被指定为统筹者");
    expect(appointed.find(".channel-notice").classes()).toContain("info");

    const elected = mountNotice(ev("channel_coordinator", { action: "elected", agent_id: "ag2" }));
    expect(elected.find(".channel-notice-text").text()).toBe("审校 当选统筹者");
    expect(elected.find(".channel-notice").classes()).toContain("info");

    const removed = mountNotice(ev("channel_coordinator", { action: "removed", agent_id: "ag1" }));
    expect(removed.find(".channel-notice-text").text()).toBe("写手 的统筹位已罢免");
    expect(removed.find(".channel-notice").classes()).toContain("muted");

    const fo = mountNotice(ev("channel_coordinator", { action: "failed-over", agent_id: "ag2" }));
    expect(fo.find(".channel-notice-text").text()).toBe("统筹者故障，换帅至 审校");
    expect(fo.find(".channel-notice").classes()).toContain("muted");
  });

  it("名字回退：成员已删（agent store 查无）→「已退出成员」", () => {
    const w = mountNotice(ev("channel_mention", { from_agent_id: "ag-gone", to_agent_id: "ag1", hop_index: 1, chain_remaining: 0 }));
    expect(w.find(".channel-notice-text").text()).toBe("已退出成员 点名 写手 接力");
  });
});
