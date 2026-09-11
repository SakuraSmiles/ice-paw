<!--
  ChannelNotice — 频道行为事件通知条（选举/统筹位/点名路由，内联消息流）

  渲染 channel_election / channel_coordinator / channel_mention 三 kind 的
  SessionEvent（useChannel 提供，按 created_at 与消息组交错）。事件是行为
  事实非消息——不占气泡位，居中细行呈现（date-divider 的信息加强版）。

  agent 名解析：优先 payload 内名字（from_agent_id 只有 id，走 agent store
  兜底「已退出成员」）——提及自身无名字快照的字段只补 id 语义，用 store
  名字解析 + 诚实回退。

  Props: event（三 kind 之一）
-->
<script setup lang="ts">
import { computed } from "vue";
import { AtSign, Shield, Vote } from "@lucide/vue";
import { useAgentStore } from "../../stores/agent";
import { timeAgo } from "../../utils/time";
import type { SessionEvent } from "../../types";

const props = defineProps<{ event: SessionEvent }>();
const agent = useAgentStore();

/** 护栏拦截原因词表（与后端 blocked_reason 六值镜像；机理长文不进通知条） */
const BLOCKED_LABELS: Record<string, string> = {
  pair_repeat: "乒乓互 @ 拦截",
  chain_limit: "接力链达上限",
  frequency: "频率闸拦截",
  user_preempted: "用户插话，接力取消",
  ambiguous_name: "成员重名，点名歧义",
  coordinator_failed: "统筹者故障，降级处理",
};

const kind = computed(() => props.event.kind);

/** agent 名解析（频道成员可能已删——store 查无回退「已退出成员」） */
function nameOf(agentId: string | null | undefined): string {
  if (!agentId) return "";
  return agent.getById(agentId)?.name ?? "已退出成员";
}

/** 通知主体文案（按 kind 分派；文案克制、无感叹） */
const text = computed<string>(() => {
  if (kind.value === "channel_mention") {
    const p = props.event.payload as import("../../types").ChannelMentionPayload;
    if (p.blocked_reason) {
      const label = BLOCKED_LABELS[p.blocked_reason] ?? p.blocked_reason;
      const from = p.from_agent_id
        ? `${nameOf(p.from_agent_id)} @ ${nameOf(p.to_agent_id)}`
        : p.broadcast ? `广播 · ${nameOf(p.to_agent_id)}` : `点名 ${nameOf(p.to_agent_id)}`;
      return `${from}：${label}`;
    }
    // from=null 的成功派发已被 useChannel 过滤（用户自起不回显）——此处兜底
    // 分词仍按 broadcast 诚实（广播接令 ≠ 点名）
    if (!p.from_agent_id && p.broadcast) return `广播 · ${nameOf(p.to_agent_id)} 接令`;
    const from = p.from_agent_id ? nameOf(p.from_agent_id) : "用户";
    return `${from} 点名 ${nameOf(p.to_agent_id)} 接力`;
  }
  if (kind.value === "channel_election") {
    const p = props.event.payload as import("../../types").ChannelElectionPayload;
    if (p.phase === "started") return "发起了统筹者自选举";
    if (p.phase === "vote") {
      const voter = nameOf(p.vote?.voter_agent_id);
      if (!p.vote?.candidate_agent_id) return `${voter} 弃权`;
      return `${voter} 投给 ${nameOf(p.vote.candidate_agent_id)}`;
    }
    const winner = p.result?.winner_agent_id ? nameOf(p.result.winner_agent_id) : null;
    return winner ? `${winner} 当选统筹者` : "全员弃权，统筹位空缺";
  }
  // channel_coordinator
  const p = props.event.payload as import("../../types").ChannelCoordinatorPayload;
  const who = p.agent_id ? nameOf(p.agent_id) : "";
  switch (p.action) {
    case "appointed": return `${who} 被指定为统筹者`;
    case "removed": return `${who} 的统筹位已罢免`;
    case "failed-over": return `统筹者故障，换帅至 ${who || "空缺"}`;
    default: return `${who} 当选统筹者`; // elected
  }
});

const icon = computed(() => {
  if (kind.value === "channel_mention") return AtSign;
  if (kind.value === "channel_election") return Vote;
  return Shield;
});

/** 拦截/降级态走中性灰；当选/指定走主色（Crown 只做 icon 变体不引入新色） */
const tone = computed(() => {
  if (kind.value === "channel_mention") {
    const p = props.event.payload as import("../../types").ChannelMentionPayload;
    return p.blocked_reason ? "muted" : "info";
  }
  if (kind.value === "channel_election") {
    const p = props.event.payload as import("../../types").ChannelElectionPayload;
    return p.phase === "result" && p.result?.winner_agent_id ? "info" : "muted";
  }
  const p = props.event.payload as import("../../types").ChannelCoordinatorPayload;
  return p.action === "appointed" || p.action === "elected" ? "info" : "muted";
});

const timeLabel = computed(() => timeAgo(props.event.created_at));
</script>

<template>
  <div :class="['channel-notice', tone]" :title="text">
    <component :is="icon" :size="12" aria-hidden="true" />
    <span class="channel-notice-text">{{ text }}</span>
    <span class="channel-notice-time">{{ timeLabel }}</span>
  </div>
</template>

<style scoped>
/* 居中细行通知条（不占气泡位；与 date-divider 同一呈现家族但带图标与时间） */
.channel-notice {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  max-width: 100%;
  padding: 2px var(--ip-spacing-2);
  margin: var(--ip-spacing-1) auto;
}
.channel-notice svg { display: inline-block; flex-shrink: 0; }
.channel-notice.info svg { color: var(--ip-primary-600); }
.channel-notice.muted svg { color: var(--ip-color-text-tertiary); }
.channel-notice-text {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-secondary);
  overflow: hidden; white-space: nowrap; text-overflow: ellipsis;
}
.channel-notice.info .channel-notice-text { color: var(--ip-primary-600); }
.channel-notice-time {
  flex-shrink: 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
</style>
