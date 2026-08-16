<script setup lang="ts">
// 会话级 token 预算胶囊 — chat:budget 事件驱动的用量 HUD。
// 两处挂载：发送中（cursor-bar 内）与轮末（finish-reason 行），持续可见
// 「已用 X / 上限 Y」；续期后上限自动抬升；用量 ≥80% 上限时转 warn 态提醒。
import { computed } from "vue";
import type { ChatBudgetPayload } from "../../types";
import { formatTokenCount } from "../../utils/format";

const props = defineProps<{ budget: ChatBudgetPayload }>();

const usagePct = computed(() =>
  props.budget.effective_cap > 0
    ? props.budget.cumulative_tokens / props.budget.effective_cap
    : 0,
);
/** 用量达 80%（TokenWindowStage 同款水位语义）转 warn 态 */
const warn = computed(() => usagePct.value >= 0.8);
const title = computed(() => {
  const base = `本回合累计 token：${props.budget.cumulative_tokens} / 上限 ${props.budget.effective_cap}（毛成本 Σ prompt+completion）`;
  return props.budget.max_renewals === 0
    ? `${base}；agent.yaml 显式上限 = 硬上限，不自动续期`
    : `${base}；可自动续期 ${props.budget.max_renewals} 次`;
});
</script>

<template>
  <span class="budget-pill" :class="{ warn }" :title="title">
    预算 {{ formatTokenCount(props.budget.cumulative_tokens) }} /
    {{ formatTokenCount(props.budget.effective_cap) }}
    <span v-if="props.budget.renewal_index > 0" class="renewed">
      （已续期 {{ props.budget.renewal_index }}/{{ props.budget.max_renewals }}）
    </span>
  </span>
</template>

<style scoped>
.budget-pill {
  display: inline-block;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-primary-tint-text);
  padding: 2px 10px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-color-primary-tint-bg);
  white-space: nowrap;
}
/* 80% 水位：换 soft 底色加重提醒（仍非错误——续期/继续都可恢复） */
.budget-pill.warn {
  background: var(--ip-color-primary-soft-bg);
  font-weight: 600;
}
.renewed {
  opacity: 0.85;
}
</style>
