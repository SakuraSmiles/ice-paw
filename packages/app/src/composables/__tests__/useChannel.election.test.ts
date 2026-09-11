// useChannel.election.test.ts — 频道选举聚合卡数据层锁定（生产反馈②）。
// 锁定语义（loadChannelNotices → applyEvents 三视图）：
// - electionCards：channel_election 按 turn_id 归组一届（started 锚 createdAt /
//   vote 累积 / result 覆写），多届互不混票
// - electionVoteIds：election: 前缀 turn 的 assistant_message message_id 全集
//   （ChatMessages 据此跳过投票行气泡）
// - notices 过滤三道：channel_election 三 phase 全进卡不出通知行；与卡 result
//   配对的 channel_coordinator(elected) 抑制（防「当选」双显）；用户自起首跳
//   派发（from=null 且未拦截——真 @ / 广播接令）不回显（答案气泡带身份头，
//   回显即噪音）；appointed / removed / failed-over 与无卡 elected 照常保留
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { bridge } from "../../api/bridge";
import { useChannel, loadChannelNotices } from "../useChannel";
import { useChatStore } from "../../stores/chat";
import type { SessionEvent } from "../../types";

vi.mock("../../api/bridge", () => ({
  bridge: {
    trajectory: { listEvents: vi.fn().mockResolvedValue([]) },
  },
}));

const mockList = vi.mocked(bridge.trajectory.listEvents);

let seq = 0;
/** 判别联合（kind+payload 成对）在测试构造侧整体断言——kind 是变量时无法逐对窄化 */
function ev(kind: SessionEvent["kind"], payload: unknown, createdAt: string, extra: Partial<SessionEvent> = {}): SessionEvent {
  seq += 1;
  return {
    id: seq, session_id: "ch1", seq, kind, actor: "user",
    turn_id: null, message_id: null, payload, created_at: createdAt,
    ...extra,
  } as SessionEvent;
}

/** 一届选举事件流（started → vote×N → result），turn_id 同 "election:{id}" */
function electionEvents(
  eid: string,
  votes: Array<{ voter: string; candidate: string | null; reason?: string }>,
  winner: string | null,
  createdAt: string,
): SessionEvent[] {
  const turn = `election:${eid}`;
  const out: SessionEvent[] = [
    ev("channel_election", { v: 1, phase: "started", vote: null, result: null }, createdAt, { turn_id: turn }),
  ];
  let i = 0;
  for (const v of votes) {
    i += 1;
    out.push(ev(
      "channel_election",
      { v: 1, phase: "vote", vote: { voter_agent_id: v.voter, candidate_agent_id: v.candidate, reason: v.reason ?? null }, result: null },
      `2026-09-11T10:00:0${i}Z`,
      { turn_id: turn },
    ));
    // 投票行事件（assistant_message，message_id = 物化票面行）
    out.push(ev("assistant_message", {}, `2026-09-11T10:00:0${i}Z`, { turn_id: turn, message_id: `vote-${eid}-${i}` }));
  }
  out.push(ev(
    "channel_election",
    { v: 1, phase: "result", vote: null, result: { tally: [], winner_agent_id: winner, tie_break: null } },
    "2026-09-11T10:00:30Z",
    { turn_id: turn },
  ));
  return out;
}

async function loadWith(evts: SessionEvent[]) {
  mockList.mockResolvedValue(evts);
  await loadChannelNotices("ch1");
}

describe("useChannel 选举聚合卡", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockList.mockReset().mockResolvedValue([]);
    const chat = useChatStore();
    chat.conversations = [{
      id: "ch1", agent_id: "ag1", title: "项目频道", pinned: false,
      created_at: "2026-09-10 00:00:00", updated_at: "2026-09-10 00:00:00",
      project_id: "p1", kind: "channel", archived_at: null,
    }];
    chat.activeConvId = "ch1";
  });

  it("一届选举聚成一张卡：started 锚时间 / vote 保序累积 / result 覆写", async () => {
    await loadWith(electionEvents(
      "e1",
      [
        { voter: "ag1", candidate: "ag2" },
        { voter: "ag2", candidate: "ag2" },
        { voter: "ag3", candidate: null, reason: "模型返回空（弃权）" },
      ],
      "ag2",
      "2026-09-11T10:00:00Z",
    ));

    const { electionCards, electionVoteIds } = useChannel();
    expect(electionCards.value.length).toBe(1);
    const card = electionCards.value[0];
    expect(card.key).toBe("election:e1");
    expect(card.createdAt).toBe("2026-09-11T10:00:00Z");
    expect(card.votes.map((v) => v.voter_agent_id)).toEqual(["ag1", "ag2", "ag3"]);
    expect(card.votes[2].candidate_agent_id).toBeNull();
    expect(card.result?.winner_agent_id).toBe("ag2");
    // 投票行 id 全集（3 条 assistant_message）
    expect([...electionVoteIds.value]).toEqual(["vote-e1-1", "vote-e1-2", "vote-e1-3"]);
  });

  it("多届选举互不混票（turn_id 分组）；进行中无 result → null", async () => {
    await loadWith([
      ...electionEvents("e1", [{ voter: "ag1", candidate: "ag2" }], "ag2", "2026-09-11T09:00:00Z"),
      // 第二届进行中（无 result 事件）
      ev("channel_election", { v: 1, phase: "started", vote: null, result: null }, "2026-09-11T11:00:00Z", { turn_id: "election:e2" }),
      ev("channel_election", { v: 1, phase: "vote", vote: { voter_agent_id: "ag1", candidate_agent_id: "ag1", reason: null }, result: null }, "2026-09-11T11:00:05Z", { turn_id: "election:e2" }),
    ]);

    const cards = useChannel().electionCards.value;
    expect(cards.length).toBe(2);
    expect(cards[0].key).toBe("election:e1");
    expect(cards[0].votes.length).toBe(1);
    expect(cards[1].key).toBe("election:e2");
    expect(cards[1].result).toBeNull();
    expect(cards[1].votes.length).toBe(1);
  });

  it("通知过滤：选举进卡；配对 elected 抑制；用户自起派发不回显、接力/拦截保留", async () => {
    await loadWith([
      ...electionEvents("e1", [{ voter: "ag1", candidate: "ag2" }], "ag2", "2026-09-11T10:00:00Z"),
      // 与卡配对的 elected → 抑制（防「当选」双显）
      ev("channel_coordinator", { v: 1, action: "elected", agent_id: "ag2", reason: "自选举产出" }, "2026-09-11T10:00:31Z"),
      // 用户治理动作 → 保留
      ev("channel_coordinator", { v: 1, action: "appointed", agent_id: "ag1", reason: null }, "2026-09-11T10:01:00Z"),
      // 用户自起首跳派发（广播接令 / 真 @ 点名）→ 不回显：答案气泡已带身份头，
      // 系统再通知一遍即噪音（生产实案：每条广播都出「用户 点名 X 接力」）
      ev("channel_mention", { v: 1, from_agent_id: null, to_agent_id: "ag1", hop_index: 1, chain_remaining: 0, broadcast: true, blocked_reason: null }, "2026-09-11T10:01:30Z"),
      ev("channel_mention", { v: 1, from_agent_id: null, to_agent_id: "ag2", hop_index: 1, chain_remaining: 0, blocked_reason: null }, "2026-09-11T10:01:40Z"),
      // 成员接力（from 有值）→ 保留（谁派发谁不显而易见）
      ev("channel_mention", { v: 1, from_agent_id: "ag2", to_agent_id: "ag1", hop_index: 2, chain_remaining: 0, blocked_reason: null }, "2026-09-11T10:02:00Z"),
      // 护栏拦截 → 保留（解释「为什么没人应答」）
      ev("channel_mention", { v: 1, from_agent_id: null, to_agent_id: "ag1", hop_index: 0, chain_remaining: 0, broadcast: true, blocked_reason: "user_preempted" }, "2026-09-11T10:02:10Z"),
    ]);

    const ns = useChannel().notices.value;
    expect(ns.length).toBe(3);
    expect(ns.map((n) => n.kind)).toEqual(["channel_coordinator", "channel_mention", "channel_mention"]);
    expect((ns[0].payload as { action: string }).action).toBe("appointed");
    expect((ns[1].payload as { from_agent_id: string }).from_agent_id).toBe("ag2");
    expect((ns[2].payload as { blocked_reason: string }).blocked_reason).toBe("user_preempted");
  });

  it("无卡 elected（窗口裁掉了对应选举）→ 照常保留通知行", async () => {
    await loadWith([
      ev("channel_coordinator", { v: 1, action: "elected", agent_id: "ag9", reason: null }, "2026-09-11T10:00:00Z"),
    ]);
    expect(useChannel().notices.value.length).toBe(1);
  });

  it("非频道会话 → 三视图清空", async () => {
    const chat = useChatStore();
    chat.conversations[0].kind = "chat";
    await loadChannelNotices("ch1");
    const { notices, electionCards, electionVoteIds } = useChannel();
    expect(notices.value.length).toBe(0);
    expect(electionCards.value.length).toBe(0);
    expect(electionVoteIds.value.size).toBe(0);
    expect(mockList).not.toHaveBeenCalled();
  });
});
