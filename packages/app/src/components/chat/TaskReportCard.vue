<script setup lang="ts">
// TaskReportCard.vue — 异步委派完成回投的呈现（批 P2-1b 方案 A，2026-11-12 拍板）
//
// 回投数据层不动（inbox 消费物化 user 消息——单写者仲裁/append-only 全保持），
// 只改呈现：来源是本会话子会话的回投（isTaskReport 判定）从「用户气泡 + 跨会话
// 标注头」改为**全宽系统汇报卡**——它是任务事实不是用户发言，占用户侧气泡
// 会让人误以为自己发过这条（用户手测原话：感官差）。
// 视觉与 DelegationCard 同族（左竖线强调边 + soft 底）；异常终态换 danger 竖线。
// 后续用户会重设计委派任务卡的完整表现形式（本组件是临时形态，承担语义修正）。
import { computed } from "vue";
import { CheckCircle2, AlertTriangle } from "@lucide/vue";
import type { IncomingInfo } from "../../utils/crossSession";
import { TASK_REPORT_DONE_PREFIX, TASK_REPORT_ABNORMAL_PREFIX } from "../../utils/crossSession";
import EntityAvatar from "../common/EntityAvatar.vue";

const props = defineProps<{
  info: IncomingInfo;
  /** 来源 agent id（子会话行有；头像取数用，可空走名字兜底） */
  agentId?: string | null;
  agentImage?: string | null;
}>();

const emit = defineEmits<{ (e: "open-task"): void }>();

const abnormal = computed(() => props.info.body.startsWith(TASK_REPORT_ABNORMAL_PREFIX));
const done = computed(() => props.info.body.startsWith(TASK_REPORT_DONE_PREFIX));
</script>

<template>
  <div class="tr-card" :class="{ abnormal }">
    <div class="tr-head">
      <EntityAvatar class="tr-avatar" :name="info.agentName" :image="agentImage ?? null" size="sm" />
      <component :is="abnormal ? AlertTriangle : CheckCircle2" :size="14" class="tr-glyph" :class="{ abnormal }" aria-hidden="true" />
      <span class="tr-title">
        {{ abnormal ? "任务异常结束" : (done ? "任务完成" : "任务回报") }} · {{ info.agentName }}
      </span>
      <button class="tr-open" title="打开任务会话（默认轨迹视图）" @click.stop="emit('open-task')">打开任务</button>
    </div>
    <div class="tr-body">{{ info.body }}</div>
  </div>
</template>

<style scoped>
/* DelegationCard 同族视觉（左竖线强调边 + soft 底）；异常终态 danger 竖线 */
.tr-card {
  border: 1px solid var(--ip-color-border-default);
  border-left: 3px solid var(--ip-success-base);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-secondary);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1_5);
  max-width: 560px;
  margin: 2px 0;
}
.tr-card.abnormal { border-left-color: var(--ip-danger-base); }
.tr-head { display: flex; align-items: center; gap: var(--ip-spacing-1_5); }
.tr-avatar { flex-shrink: 0; }
.tr-glyph { flex-shrink: 0; color: var(--ip-success-text); display: inline-block; }
.tr-glyph.abnormal { color: var(--ip-danger-text); }
.tr-title { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.tr-open {
  margin-left: auto; border: none; background: none; color: var(--ip-primary-600);
  font-size: var(--ip-text-caption-size); cursor: pointer;
  padding: 2px var(--ip-spacing-1_5); border-radius: var(--ip-radius-sm);
}
.tr-open:hover { background: var(--ip-primary-soft-bg, rgba(0, 0, 0, 0.05)); }
.tr-body {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 220px;
  overflow-y: auto;
}
</style>
