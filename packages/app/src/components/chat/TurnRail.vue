<!--
  TurnRail — 轮次导航条（UX #5 v2：定容滑动窗口）

  消息区右侧纵向轨道：一轮 = 一小线，但任意会话规模下最多同时显示
  RAIL_WINDOW 格——窗口围绕当前视位轮居中（activeTurn 居中锚定，
  边缘钳制），窗口外轮次用上下「⋮」省略号整窗翻页，轨道上滚轮半窗
  微调（微调只挪窗口不动内容，点任意轮/内容滚动即回锚居中）。
  顶部位置徽标「N/M」补回全局位置感（窗口化丢失的信息）。

  窗口状态在组件内：scrubFrom（null=自动居中，数值=手动微调偏移）。
  数据在父级（useTurnRail 锚点 + 视位侦测）——本组件纯展示/转发：
  Props: anchors（全量锚点）/ activeTurn（视口顶所在轮，null=未知）/ showLatest
  Emits: jump(messageId) / latest()
-->
<script setup lang="ts">
import { ref, computed, watch } from "vue";
import type { TurnAnchor } from "../../types";
import { RAIL_WINDOW, autoWindowStart, buildTurnWindow, type TurnTick } from "../../composables/useTurnRail";
import { formatDateLabel, formatTime } from "../../utils/time";

const props = defineProps<{ anchors: TurnAnchor[]; activeTurn: number | null; showLatest: boolean }>();
const emit = defineEmits<{ jump: [messageId: string]; latest: [] }>();

// ---- 窗口状态：null=自动（activeTurn 居中）；数值=手动微调起点 ----
// activeTurn 变化（点轮/内容滚动/新轮到达）即脱离手动模式回锚——
// 窗口永远跟着用户实际所在轮走；换会话锚点全换同样复位。
const scrubFrom = ref<number | null>(null);
watch(() => props.activeTurn, () => { scrubFrom.value = null; });
watch(() => props.anchors, () => { scrubFrom.value = null; wheelAcc = 0; });

const win = computed(() => {
  const total = props.anchors.length;
  const from = scrubFrom.value ?? autoWindowStart(total, props.activeTurn, RAIL_WINDOW);
  return buildTurnWindow(props.anchors, from, RAIL_WINDOW);
});

// ---- 滚轮微调：窗口半窗步进（不动内容）；高频小事件按累计量合并 ----
let wheelAcc = 0;
function onWheel(e: WheelEvent) {
  const total = props.anchors.length;
  if (total <= RAIL_WINDOW) return; // 全部可见，无需微调
  wheelAcc += e.deltaY;
  while (Math.abs(wheelAcc) >= 50) {
    const dir = wheelAcc > 0 ? 1 : -1;
    wheelAcc -= dir * 50;
    shiftWindow(dir);
  }
}
function shiftWindow(dir: 1 | -1) {
  const total = props.anchors.length;
  const maxStart = Math.max(1, total - RAIL_WINDOW + 1);
  const next = Math.min(Math.max(1, win.value.from + dir * Math.ceil(RAIL_WINDOW / 2)), maxStart);
  if (next !== win.value.from) scrubFrom.value = next;
}

// ---- 省略号：整窗翻页（跳内容到目标轮，activeTurn 变化后窗口自动居中过去） ----
function pageJump(dir: 1 | -1) {
  const total = props.anchors.length;
  if (total === 0) return;
  const target = dir > 0
    ? Math.min(total, win.value.from - 1 + RAIL_WINDOW)
    : Math.max(1, win.value.from - RAIL_WINDOW);
  emit("jump", props.anchors[target - 1].message_id);
}

// ---- tooltip：跟随 hovered tick 定位（轨道左侧展开，盖在内容列上） ----
const hover = ref<{ top: number; tick: TurnTick } | null>(null);
function onTickEnter(t: TurnTick, e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  hover.value = { top: el.offsetTop + el.offsetHeight / 2, tick: t };
}
function onTickLeave() {
  hover.value = null;
}

/** tooltip 时间 = 日期标签 + HH:MM（跨日跳转时光看 HH:MM 会误导） */
function tickTime(iso: string): string {
  const d = formatDateLabel(iso);
  return d ? `${d} ${formatTime(iso)}` : formatTime(iso);
}
</script>

<template>
  <nav v-if="win.total >= 2" class="turn-rail" aria-label="轮次导航">
    <!-- 位置徽标：当前轮 / 总轮数（窗口化后唯一的全局位置指示） -->
    <div class="turn-pos">
      <span class="turn-pos-cur" :class="{ known: activeTurn !== null }">{{ activeTurn ?? "–" }}</span>
      <span class="turn-pos-sep">/</span>
      <span class="turn-pos-total">{{ win.total }}</span>
    </div>

    <!-- 上方省略号：窗口之上还有轮次，点击整窗前翻 -->
    <button v-if="win.hasPrev" class="turn-more" :title="`往前翻 ${win.from - 1} 轮`" @click="pageJump(-1)">⋮</button>

    <div class="turn-rail-track" @mouseleave="onTickLeave" @wheel.prevent="onWheel">
      <button
        v-for="t in win.ticks"
        :key="t.turn"
        class="turn-tick"
        :class="{ active: activeTurn === t.turn }"
        :title="`第 ${t.turn} 轮 · 点击跳转`"
        @mouseenter="onTickEnter(t, $event)"
        @click="emit('jump', t.messageId)"
      />

      <!-- hover tooltip（轮号 + 首行预览 + 时间） -->
      <div v-if="hover" class="turn-tip" :style="{ top: `${hover.top}px` }">
        <span class="turn-tip-no">第 {{ hover.tick.turn }} 轮</span>
        <span class="turn-tip-preview">{{ hover.tick.preview || "(无文本)" }}</span>
        <span class="turn-tip-time">{{ tickTime(hover.tick.createdAt) }}</span>
      </div>
    </div>

    <!-- 下方省略号：窗口之下还有轮次，点击整窗后翻 -->
    <button v-if="win.hasNext" class="turn-more" :title="`往后翻 ${win.total - (win.from - 1 + RAIL_WINDOW)} 轮`" @click="pageJump(1)">⋮</button>

    <!-- 「跳到最新」：轨道底部（下三角，样式区别于轮次线） -->
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
/* 轨道：右侧预留带内，垂直居中、贴内容列右缘。v2 窗口化后高度天然有界
   （≤13 tick × 20px + 徽标/省略号/跳最新），不再需要内容驱动定高；
   矮视口下 max-height 兜底裁切（轨道不滚动——滚轮是窗口微调手势） */
.turn-rail {
  position: absolute;
  right: 10px;
  top: 50%;
  transform: translateY(-50%);
  max-height: 80%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  z-index: 3;
}

/* 位置徽标：N/M，等宽数字防跳动；未知轮次时当前位灰显 */
.turn-pos {
  display: flex;
  align-items: baseline;
  gap: 2px;
  font-family: var(--ip-font-mono, monospace);
  font-size: 10px;
  line-height: 1;
  color: var(--ip-color-text-tertiary);
  user-select: none;
}
.turn-pos-cur { color: var(--ip-color-text-disabled); }
.turn-pos-cur.known { color: var(--ip-primary-600); font-weight: var(--ip-font-weight-semibold); }
.turn-pos-sep { opacity: 0.6; }

/* 省略号（整窗翻页）：竖排三点，弱色 → hover 主色 */
.turn-more {
  width: 22px;
  height: 16px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--ip-color-text-disabled);
  font-size: 12px;
  line-height: 1;
  cursor: pointer;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.turn-more:hover { color: var(--ip-primary-600); }

/* tick 轨：固定行高（窗口容量定轨高），微调时整轨重渲染 */
.turn-rail-track {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 22px;
  flex: 0 1 auto;
  overflow: hidden;
}
.turn-tick {
  flex: 0 0 auto;
  width: 22px;
  height: 20px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}
/* 线本体：细弱底色 → hover 加宽提亮；当前轮主色常亮 */
.turn-tick::after {
  content: "";
  width: 10px;
  height: 2px;
  border-radius: 1px;
  background: var(--ip-color-border-strong);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
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
