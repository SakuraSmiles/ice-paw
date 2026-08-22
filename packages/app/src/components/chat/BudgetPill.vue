<script setup lang="ts">
// 会话级 token 预算 HUD — chat:budget 事件驱动。
// 挂载（2026-08-22 定位拍板）：ChatInput 输入框底部工具栏中间位——有预算数据
// 即占中（快捷键提示让位），发送中与轮末同一位置持续可见（budget 在 store
// 存活到下一次 send/切会话）。
// 形态：环形进度（灰轨绿芯，12 点起顺时针，80% 处弱刻度点）+「已用 X / 上限 Y」
// 数字——预算本质是比值，环的填充度一眼读出余量；用量 ≥80% 越过刻度转 warn 态
// （芯环加深 + 字重 600），续期后上限自动抬升（环随之回落）。
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
/** 填充比（%）；钳 100% 防瞬时超限撑满整环 */
const fillPct = computed(() => Math.min(100, usagePct.value * 100));
/** 环几何：viewBox 20 系 r=8 → C=2πr；offset = C ×(1−填充比)（12 点起顺时针填充） */
const RING_C = 2 * Math.PI * 8;
const ringOffset = computed(() => RING_C * (1 - fillPct.value / 100));
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
    <svg class="budget-ring" viewBox="0 0 20 20" aria-hidden="true">
      <circle class="ring-track" cx="10" cy="10" r="8" />
      <circle
        class="ring-fill"
        cx="10"
        cy="10"
        r="8"
        :stroke-dasharray="RING_C"
        :stroke-dashoffset="ringOffset"
      />
      <!-- 80% 水位刻度点（12 点起顺时针 288° 处，落在轨道圆上） -->
      <circle class="budget-tick" cx="2.39" cy="7.53" r="1.1" />
    </svg>
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
/* 环形进度：灰轨绿芯，12 点起顺时针；-90° 旋转把 dash 起点转到顶部。
   尺寸 = 文字的 ~1.17em（caption 12px → 环 14px）：圆对同边长方形显小，
   略大于字面才等重；等高（12px）则描边/刻度细到消隐 */
.budget-ring {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  transform: rotate(-90deg);
}
.ring-track {
  fill: none;
  stroke: var(--ip-color-bg-tertiary);
  stroke-width: 3;
}
.ring-fill {
  fill: none;
  stroke: var(--ip-primary-500);
  stroke-width: 3;
  stroke-linecap: round;
  transition: stroke-dashoffset var(--ip-duration-fast) var(--ip-ease-out);
}
.budget-tick {
  fill: var(--ip-color-border-default);
  opacity: 0.6;
}
/* 80% 水位：芯环加深 + 字重提醒（仍非错误——续期/继续都可恢复） */
.budget-pill.warn { font-weight: 600; }
.budget-pill.warn .ring-fill { stroke: var(--ip-color-primary-tint-text); }
/* 命中率与续期计数同为次级信息，弱化视觉 */
.cached {
  opacity: 0.85;
}
.renewed {
  opacity: 0.85;
}
</style>
