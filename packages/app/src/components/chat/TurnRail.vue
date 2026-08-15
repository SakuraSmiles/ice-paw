<!--
  TurnRail — 轮次导航条（UX #5，纯导航 v1）

  消息区右侧纵向轨道：一轮 = 一小线（超 RAIL_CAPACITY 轮自动聚合为组，
  更高的 tick 暗示密度）；hover 出 tooltip（轮号 + 用户消息首行 + 时间）；
  点击跳该轮（窗口外由父级补页定位）；当前视位轮高亮；底部「跳到最新」
  （原独立按钮退位至此，仅非跟随态显示）。

  状态与数据全在父级（useTurnRail + 视位侦测）——本组件纯展示/转发：
  Props: buckets（分桶结果）/ activeTurn（视口顶所在轮，null=未知）/ showLatest
  Emits: jump(messageId) / latest()
-->
<script setup lang="ts">
import { ref } from "vue";
import type { TurnBucket } from "../../composables/useTurnRail";
import { formatDateLabel, formatTime } from "../../utils/time";

defineProps<{ buckets: TurnBucket[]; activeTurn: number | null; showLatest: boolean }>();
const emit = defineEmits<{ jump: [messageId: string]; latest: [] }>();

// ---- tooltip：跟随 hovered tick 定位（轨道左侧展开，盖在内容列上） ----
const hover = ref<{ top: number; bucket: TurnBucket } | null>(null);
function onTickEnter(b: TurnBucket, e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  hover.value = { top: el.offsetTop + el.offsetHeight / 2, bucket: b };
}
function onTickLeave() {
  hover.value = null;
}

function bucketLabel(b: TurnBucket): string {
  return b.from === b.to ? `第 ${b.from} 轮` : `第 ${b.from}–${b.to} 轮`;
}

/** tooltip 时间 = 日期标签 + HH:MM（跨日跳转时光看 HH:MM 会误导） */
function bucketTime(iso: string): string {
  const d = formatDateLabel(iso);
  return d ? `${d} ${formatTime(iso)}` : formatTime(iso);
}
</script>

<template>
  <nav v-if="buckets.length >= 2" class="turn-rail" aria-label="轮次导航">
    <div class="turn-rail-track" @mouseleave="onTickLeave">
      <button
        v-for="b in buckets"
        :key="b.from"
        class="turn-tick"
        :class="{ group: b.to > b.from, active: activeTurn !== null && activeTurn >= b.from && activeTurn <= b.to }"
        :title="`${bucketLabel(b)} · 点击跳转`"
        @mouseenter="onTickEnter(b, $event)"
        @click="emit('jump', b.messageId)"
      />

      <!-- hover tooltip（轮号 + 首行预览 + 时间） -->
      <div v-if="hover" class="turn-tip" :style="{ top: `${hover.top}px` }">
        <span class="turn-tip-no">{{ bucketLabel(hover.bucket) }}</span>
        <span class="turn-tip-preview">{{ hover.bucket.preview || "(无文本)" }}</span>
        <span class="turn-tip-time">{{ bucketTime(hover.bucket.createdAt) }}</span>
      </div>
    </div>

    <!-- 「跳到最新」：退位到轨道底部（下三角，样式区别于轮次线） -->
    <Transition name="fade-up">
      <button v-if="showLatest" class="turn-latest" title="回到底部并跟随最新" @click="emit('latest')">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>
    </Transition>
  </nav>
</template>

<style scoped>
/* 轨道：右侧预留带内，垂直居中、贴内容列右缘。高度不抢满——目录条不是滚动条 */
.turn-rail {
  position: absolute;
  right: 10px;
  top: 50%;
  transform: translateY(-50%);
  max-height: 72%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  z-index: 3;
}
/* tick 轨：flex 均布（justify-between 让首尾贴边，中间等距） */
.turn-rail-track {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: center;
  width: 22px;
}
.turn-tick {
  flex: 0 0 auto;
  width: 22px;
  height: 9px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}
/* 线本体：细弱底色 → hover 加宽提亮；聚合组更高（密度暗示）；当前轮主色常亮 */
.turn-tick::after {
  content: "";
  width: 10px;
  height: 2px;
  border-radius: 1px;
  background: var(--ip-color-border-strong);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.turn-tick.group::after { height: 4px; }
.turn-tick:hover::after { width: 16px; background: var(--ip-primary-500); }
.turn-tick.active::after { width: 16px; background: var(--ip-primary-500); }
.turn-tick.active:hover::after { width: 18px; }
/* 暗色下提亮 */
[data-theme="dark"] .turn-tick::after { background: var(--ip-color-border-default); }

/* tooltip：轨道左侧展开（盖内容列），跟随 tick 垂直位置；单行省略防长文撑爆 */
.turn-tip {
  position: absolute;
  right: calc(100% + 8px);
  transform: translateY(-50%);
  display: flex;
  flex-direction: column;
  gap: 2px;
  max-width: 300px;
  padding: 8px 12px;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md);
  pointer-events: none;
  white-space: nowrap;
}
.turn-tip-no { font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-primary-600); }
.turn-tip-preview {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
}
.turn-tip-time { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

/* 「跳到最新」：下三角小圆钮（区别于轮次线的视觉语系） */
.turn-latest {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  flex-shrink: 0;
  border-radius: var(--ip-radius-full, 999px);
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-elevated);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  box-shadow: var(--ip-shadow-sm);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.turn-latest:hover { color: var(--ip-primary-600); border-color: var(--ip-primary-400); }

/* 入场（与原 scroll-bottom-btn 同款 fade-up） */
.fade-up-enter-active, .fade-up-leave-active { transition: opacity 0.18s ease, transform 0.18s ease; }
.fade-up-enter-from, .fade-up-leave-to { opacity: 0; transform: translateY(4px); }
</style>
