<!--
  ChannelElectionCard — 频道统筹者选举聚合卡（内联消息流）

  一届选举（started → vote×N → result，同 turn_id "election:{uuid}"）聚合为
  一张卡，取代零散通知行 + 投票行气泡（生产反馈②：明明是选举一件事，零散
  信息重复出现）。聚合只发生在渲染前（useChannel.applyEvents）——数据层 /
  轨迹 / append-only 事件零改，与事件日志的契合保持原样。

  成员名/头像：agent store 解析（成员可能已删 → EntityAvatar 名字回退 +
  「已退出成员」）。投票 reason 可能是凭据/请求错误长文——单行省略 + title
  悬浮全文。

  Props: card（useChannel.ElectionCard）
-->
<script setup lang="ts">
import { computed } from "vue";
import { Shield, Vote } from "@lucide/vue";
import { useAgentStore } from "../../stores/agent";
import { timeAgo } from "../../utils/time";
import EntityAvatar from "../common/EntityAvatar.vue";
import type { ElectionCard } from "../../composables/useChannel";

const props = defineProps<{ card: ElectionCard }>();
const agent = useAgentStore();

/** 成员档案（可能已删：name 回退「已退出成员」，头像交给 EntityAvatar 名字兜底） */
function profileOf(id: string | null | undefined) {
  if (!id) return null;
  const a = agent.getById(id);
  return { name: a?.name ?? "已退出成员", image: a?.avatar ?? null };
}

/** 逐票行视图（含弃权票——弃权原因也是选举事实） */
const voteRows = computed(() =>
  props.card.votes.map((v) => ({
    key: v.voter_agent_id + "-" + v.reason,
    voter: profileOf(v.voter_agent_id),
    candidate: profileOf(v.candidate_agent_id),
    abstained: !v.candidate_agent_id,
    reason: v.reason ?? null,
  })),
);

const winner = computed(() => profileOf(props.card.result?.winner_agent_id));
const allAbstained = computed(
  () => props.card.result != null && props.card.result.winner_agent_id == null,
);
const tieBreak = computed(() => props.card.result?.tie_break === "joined_at");

const timeLabel = computed(() => timeAgo(props.card.createdAt));
</script>

<template>
  <div class="election-card">
    <div class="election-head">
      <Vote :size="14" class="election-head-icon" aria-hidden="true" />
      <span class="election-head-title">统筹者选举</span>
      <span v-if="!card.result" class="election-head-status">进行中</span>
      <span class="election-head-time" :title="card.createdAt">{{ timeLabel }}</span>
    </div>

    <div v-if="voteRows.length === 0" class="election-empty">成员投票中…</div>
    <ul v-else class="election-votes">
      <li v-for="row in voteRows" :key="row.key" class="election-vote-row" :title="row.reason ?? undefined">
        <EntityAvatar v-if="row.voter" :name="row.voter.name" :image="row.voter.image" size="sm" />
        <span class="election-voter">{{ row.voter?.name ?? "已退出成员" }}</span>
        <span v-if="row.abstained" class="election-pick muted">弃权</span>
        <span v-else class="election-pick">投给 {{ row.candidate?.name ?? "已退出成员" }}</span>
        <span v-if="row.reason" class="election-reason">{{ row.reason }}</span>
      </li>
    </ul>

    <div v-if="card.result" class="election-result" :class="{ empty: allAbstained }">
      <template v-if="winner">
        <Shield :size="13" class="election-result-shield" aria-hidden="true" />
        <span class="election-result-text">{{ winner.name }} 当选统筹者</span>
        <span v-if="tieBreak" class="election-tie">平票，按最早加入裁决</span>
      </template>
      <template v-else>
        <span class="election-result-text">全员弃权，统筹位空缺</span>
      </template>
    </div>
  </div>
</template>

<style scoped>
/* 居中窄卡（比通知条重一档的聚合事实；不占气泡位，与 date-divider 同族） */
.election-card {
  max-width: 480px;
  margin: var(--ip-spacing-2) auto;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-card-radius);
  background: var(--ip-color-bg-secondary);
}
.election-head {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}
.election-head svg { display: inline-block; flex-shrink: 0; color: var(--ip-primary-600); }
.election-head-title {
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-text-caption-lh);
  font-weight: 600;
  color: var(--ip-color-text-primary);
}
/* 进行中：主色描边小胶囊（状态上屏 L2——不是可编辑字段） */
.election-head-status {
  font-size: var(--ip-text-micro-size);
  line-height: var(--ip-text-micro-lh);
  color: var(--ip-primary-600);
  border: 1px solid var(--ip-color-border-focus);
  border-radius: var(--ip-radius-full);
  padding: 0 6px;
}
.election-head-time {
  margin-left: auto;
  flex-shrink: 0;
  font-size: var(--ip-text-micro-size);
  line-height: var(--ip-text-micro-lh);
  color: var(--ip-color-text-tertiary);
}
.election-empty {
  margin-top: var(--ip-spacing-2);
  font-size: var(--ip-text-micro-size);
  line-height: var(--ip-text-micro-lh);
  color: var(--ip-color-text-tertiary);
}
.election-votes {
  list-style: none;
  margin: var(--ip-spacing-2) 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}
.election-vote-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  min-width: 0;
}
.election-voter {
  flex-shrink: 0;
  font-size: var(--ip-text-micro-size);
  line-height: var(--ip-text-micro-lh);
  color: var(--ip-color-text-primary);
}
.election-pick {
  flex-shrink: 0;
  font-size: var(--ip-text-micro-size);
  line-height: var(--ip-text-micro-lh);
  color: var(--ip-color-text-secondary);
}
.election-pick.muted { color: var(--ip-color-text-tertiary); }
/* 弃权/失败原因：单行省略 + 行级 title 悬浮全文（错误文案不截断事实） */
.election-reason {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-size: var(--ip-text-micro-size);
  line-height: var(--ip-text-micro-lh);
  color: var(--ip-color-text-tertiary);
}
.election-result {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin-top: var(--ip-spacing-2);
  padding-top: var(--ip-spacing-2);
  border-top: 1px solid var(--ip-color-border-default);
}
.election-result-shield { display: inline-block; flex-shrink: 0; color: var(--ip-primary-600); }
.election-result-text {
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-text-caption-lh);
  color: var(--ip-primary-600);
}
.election-result.empty .election-result-text { color: var(--ip-color-text-tertiary); }
.election-tie {
  font-size: var(--ip-text-micro-size);
  line-height: var(--ip-text-micro-lh);
  color: var(--ip-color-text-tertiary);
}
</style>
