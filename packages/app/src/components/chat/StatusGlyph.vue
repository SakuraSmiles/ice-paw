<!--
  StatusGlyph — 状态图标语系（2026-09-04，动效参考视频档1）

  五态（色走语义层 + 主色，勿再加新色）：
  - running：3×3 像素格顺时针点亮循环（视频签名元素）——主色，进行中是信息态非警告态
  - done：环形 + Lucide Check —— success
  - error：环形 + Lucide X —— danger
  - wait：空心环 —— warning（等待执行结果/授权）
  - pending：空心环 —— 中性（未开始，或 TaskPanel 已结束任务的诚实中性态——
    那里两态语义「不伪造 done/failed」，见 TaskPanel 头注释；用 label 覆盖 aria 文案）

  ⚠️ 不变式：
  1. 像素格基准态 opacity 0.35 全格可见——prefers-reduced-motion 全局归零（tokens §12）
     会把循环打成 0.01ms + iteration 1 落在 100% 帧，基准态即降级后的静止帧，必须可读。
  2. 全动画纯 transform/opacity（合成器友好；ChatMessages 是 content-visibility 热路径）。
  3. 循环时长唯一来源 --ip-duration-pixel（tokens.css），勿散点硬编码。
  4. 状态切换重播：v-if/v-else-if 分支切换天然重挂载（mount 动画自动重放），勿改成
     单元素 class 切换。
-->
<script setup lang="ts">
import { Check, X } from "@lucide/vue";

withDefaults(
  defineProps<{
    status: "running" | "done" | "error" | "wait" | "pending";
    /** 直径 px，默认 14 */
    size?: number;
    /** 覆盖默认 aria 文案（如 TaskPanel 已结束任务的「已结束」） */
    label?: string;
  }>(),
  { size: 14, label: undefined },
);

// 点亮序（视频：围绕边框 顺时针，最后中心）：DOM 行优先 1..9 → 点亮序 1,2,3,6,9,8,7,4,5。
// delay 步长 80ms 是动画几何（与 --ip-duration-pixel 1200ms 配套），单一常量非散点时长。
const PX_DELAYS = [0, 80, 160, 560, 640, 240, 480, 400, 320];

const DEFAULT_LABELS: Record<string, string> = {
  running: "进行中",
  done: "已完成",
  error: "出错",
  wait: "等待结果",
  pending: "未开始",
};
</script>

<template>
  <span class="status-glyph" :style="{ '--glyph-size': size + 'px' }" role="img" :aria-label="label ?? DEFAULT_LABELS[status]">
    <span v-if="status === 'running'" class="glyph-grid" aria-hidden="true">
      <span
        v-for="(d, i) in PX_DELAYS"
        :key="i"
        class="px-cell"
        :style="{ animationDelay: d + 'ms' }"
      />
    </span>
    <span v-else-if="status === 'done'" class="glyph-ring glyph-done" aria-hidden="true">
      <Check :size="size - 5" :stroke-width="3" />
    </span>
    <span v-else-if="status === 'error'" class="glyph-ring glyph-error" aria-hidden="true">
      <X :size="size - 5" :stroke-width="3" />
    </span>
    <span
      v-else
      class="glyph-ring"
      :class="status === 'wait' ? 'glyph-wait' : 'glyph-pending'"
      aria-hidden="true"
    />
  </span>
</template>

<style scoped>
.status-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: var(--glyph-size, 14px);
  height: var(--glyph-size, 14px);
}

/* ----- running：3×3 像素格（CSS Grid，循环点亮） ----- */
.glyph-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  grid-template-rows: repeat(3, 1fr);
  gap: 1px;
  width: 100%;
  height: 100%;
}
.px-cell {
  border-radius: 1px;
  background: var(--ip-primary-500);
  opacity: 0.35;
  animation: px-cell var(--ip-duration-pixel, 1.2s) var(--ip-ease-in-out) infinite;
}
@keyframes px-cell {
  0% { opacity: 0.35; transform: scale(0.85); }
  15% { opacity: 1; transform: scale(1); }
  55% { opacity: 1; }
  100% { opacity: 0.35; transform: scale(0.85); }
}

/* ----- done / error：环形 + Lucide 图标，mount 时轻 pop ----- */
.glyph-ring {
  width: 100%;
  height: 100%;
  border-radius: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  animation: glyph-pop var(--ip-duration-fast) var(--ip-ease-out);
}
.glyph-done { border: 1.5px solid var(--ip-success-base); color: var(--ip-success-base); }
.glyph-error { border: 1.5px solid var(--ip-danger-base); color: var(--ip-danger-base); }
.glyph-wait { border: 1.5px solid var(--ip-warning-base); }
.glyph-pending { border: 1.5px solid var(--ip-color-text-tertiary); }
@keyframes glyph-pop {
  from { transform: scale(0.5); opacity: 0; }
  to { transform: scale(1); opacity: 1; }
}
</style>
