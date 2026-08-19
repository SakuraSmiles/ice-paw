<script setup lang="ts">
// 会话级 token 预算 HUD — chat:budget 事件驱动。
// 两处挂载：发送中（cursor-bar 内）与轮末（finish-reason 行），持续可见。
// 形态：微型进度条（灰轨绿芯，80% 处弱刻度线）+「已用 X / 上限 Y」数字——
// 预算本质是比值，填充度一眼读出余量；用量 ≥80% 越过刻度转 warn 态
// （芯条加深 + 字重 600），续期后上限自动抬升（条随之回落）。
// 预算诚实化：数字为计费口径（缓存命中按 1/10 折扣），命中率高时涨速
// 显著慢于毛成本——旁挂「缓存命中 X%」chip 说明为什么（L2 状态上屏）。
import { computed } from "vue";
import type { ChatBudgetPayload } from "../../types";
import { formatTokenCount } from "../../utils/format";

const props = defineProps<{ budget: ChatBudgetPayload }>();

const usagePct = computed(() =>
  props.budget.effective_cap > 0
    ? props.budget.cumulative_tokens / props.budget.effective_cap
    : 0,
);
/** 填充宽（%）；钳 100% 防瞬时超限撑破轨道 */
const fillWidth = computed(() => `${Math.min(100, usagePct.value * 100)}%`);
/** 用量达 80%（TokenWindowStage 同款水位语义）转 warn 态 */
const warn = computed(() => usagePct.value >= 0.8);
/** 缓存命中率 = Σcached / Σprompt（规范语义分母）；无命中数据时隐藏 chip */
const cacheHitPct = computed(() => {
  const p = props.budget.cumulative_prompt_tokens;
  return p > 0 && props.budget.cumulative_cached_tokens > 0
    ? Math.round((props.budget.cumulative_cached_tokens / p) * 100)
    : null;
});
const title = computed(() => {
  const base = `本回合累计 token（计费口径）：${props.budget.cumulative_tokens} / 上限 ${props.budget.effective_cap}（未命中全价 + 命中 1/10 + 输出全价）`;
  return props.budget.max_renewals === 0
    ? `${base}；agent.yaml 显式上限 = 硬上限，不自动续期`
    : `${base}；可自动续期 ${props.budget.max_renewals} 次`;
});
</script>

<template>
  <span class="budget-pill" :class="{ warn }" :title="title">
    <span class="budget-bar" aria-hidden="true">
      <span class="budget-fill" :style="{ width: fillWidth }" />
      <span class="budget-tick" />
    </span>
    预算 {{ formatTokenCount(props.budget.cumulative_tokens) }} /
    {{ formatTokenCount(props.budget.effective_cap) }}
    <span v-if="cacheHitPct !== null" class="cached">
      缓存命中 {{ cacheHitPct }}%
    </span>
    <span v-if="props.budget.renewal_index > 0" class="renewed">
      （已续期 {{ props.budget.renewal_index }}/{{ props.budget.max_renewals }}）
    </span>
  </span>
</template>

<style scoped>
.budget-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
/* 微型进度条：灰轨绿芯；80% 处一道弱刻度线 = warn 水位可视锚点 */
.budget-bar {
  position: relative;
  width: 56px;
  height: 4px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-color-bg-tertiary);
  overflow: hidden;
}
.budget-fill {
  position: absolute;
  inset: 0 auto 0 0;
  border-radius: inherit;
  background: var(--ip-primary-500);
  transition: width var(--ip-duration-fast) var(--ip-ease-out);
}
.budget-tick {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 80%;
  width: 1px;
  background: var(--ip-color-border-default);
  opacity: 0.6;
}
/* 80% 水位：芯条加深 + 字重提醒（仍非错误——续期/继续都可恢复） */
.budget-pill.warn { font-weight: 600; }
.budget-pill.warn .budget-fill { background: var(--ip-color-primary-tint-text); }
/* 命中率与续期计数同为次级信息，弱化视觉 */
.cached {
  opacity: 0.85;
}
.renewed {
  opacity: 0.85;
}
</style>
