<script setup lang="ts">
// ScreenHud.vue — 屏幕共享 HUD 工具栏（批次④ 步骤 2，§4.7/§4.9）
//
// 440×44 置顶工具条窗（Rust 侧 hud.rs 创建）。渲染分两形态：
// - full：控制（暂停/恢复·终止·收起）/ 状态（时长）/ 使用者（令牌持有者+purpose）/
//   等待（队列+授予队首）/ 冲突（human_active 可视化）/ 显示器 ◀▶ / 成本（截图数）
// - mini：132×28 右上角微条——手动收起（可点回）与写避让（B7：agent 注入期间
//   自动收缩 + 点击穿透，让路给用户）共用形态；写避让结束自动回 full。
//
// 数据源：screen:channel-state 全量事件（store 订阅）+ 1s 轮询补真相——
// human_active/writing 无事件源变化（时间戳/计数翻转不广播），时长也靠轮询走秒。
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  ScreenShare, Pause, Play, X, ChevronLeft, ChevronRight,
  ChevronsRight, ChevronsLeft, MousePointer2, Camera, Loader2,
} from "@lucide/vue";
import { useScreenChannelStore } from "../../stores/screenChannel";
import { bridge } from "../../api/bridge";

const screenChannel = useScreenChannelStore();
const state = computed(() => screenChannel.state);

/** 手动收起（用户意愿；与写避让自动收缩分记——自动的必须自动恢复） */
const collapsed = ref(false);
/** 展开态 = 未收起且无写执行（写避让收缩优先级高于用户展开） */
const expanded = computed(() => !collapsed.value && !state.value.writing);
/** mini 形态下的点击穿透：仅写避让（自动收缩几秒后自愈）；手动收起必须可点回 */
const passthrough = computed(() => state.value.writing);

/** mini 形态意图 = 未展开（手动收起或写避让收缩） */
const miniForm = computed(() => !expanded.value);
// 形态切换是 Rust 侧窗口操作（resize + 穿透），前端只声明意图
watch([miniForm, passthrough], ([mini, pt]) => {
  bridge.screen.setHudForm(mini, pt).catch(() => {});
}, { immediate: true });

/** 开启时长（1s 走秒由轮询 now 驱动） */
const now = ref(Math.floor(Date.now() / 1000));
let pollTimer: number | undefined;

function durationText(): string {
  const opened = state.value.opened_at;
  if (!opened) return "0:00";
  const s = Math.max(0, now.value - opened);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h} 时 ${m} 分`;
  return `${m}:${String(sec).padStart(2, "0")}`;
}

/** 令牌持有者的展示信息（agent 名 + purpose；不在附着名单时回落 conv id） */
const holderInfo = computed(() => {
  const h = state.value.holder;
  if (!h) return null;
  const a = state.value.attached.find((c) => c.conv_id === h);
  return { name: a?.agent_name ?? h, purpose: a?.purpose ?? "" };
});

const queueHead = computed(() => state.value.queue[0] ?? null);

async function togglePause() {
  await screenChannel.setPaused(!state.value.paused).catch(() => {});
}
async function terminate() {
  await screenChannel.stop().catch(() => {});
}
async function cycleMonitor(delta: number) {
  // 命令返回即真相（bump 广播事件会再对齐一次，双通道幂等）
  screenChannel.state = await bridge.screen.cycleHudMonitor(delta).catch(() => state.value);
}
async function grantQueueHead() {
  if (queueHead.value) await screenChannel.grantTo(queueHead.value).catch(() => {});
}

onMounted(() => {
  // 主题同步：工具窗不挂 Sidebar（useTheme 所在），从主窗写的 localStorage 读
  // 应用主题（缺省跟随系统偏好——与主窗默认一致）
  const saved = localStorage.getItem("icepaw-theme");
  const dark = saved ? saved === "dark" : window.matchMedia("(prefers-color-scheme: dark)").matches;
  document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");

  // 1s 轮询：human_active/writing 无事件源 + 时长走秒
  pollTimer = window.setInterval(async () => {
    now.value = Math.floor(Date.now() / 1000);
    await screenChannel.refresh();
  }, 1000);
});

onUnmounted(() => {
  if (pollTimer) window.clearInterval(pollTimer);
});
</script>

<template>
  <!-- mini 微条（手动收起 / 写避让收缩共用形态） -->
  <div v-if="!expanded" class="hud hud-mini">
    <button class="icon-btn" :class="{ spinning: state.writing }" title="屏幕共享中">
      <Loader2 v-if="state.writing" :size="14" class="spin" />
      <ScreenShare v-else :size="14" />
    </button>
    <span class="mini-text">{{ state.writing ? "操作中" : "共享中" }}</span>
    <button v-if="!state.writing" class="icon-btn" title="展开工具栏" @click="collapsed = false">
      <ChevronsLeft :size="14" />
    </button>
  </div>

  <!-- full 工具条 -->
  <div v-else class="hud hud-full" :class="{ off: state.status === 'off' }">
    <!-- 状态：图标 + 时长/已暂停 -->
    <ScreenShare :size="15" class="lead-icon" :class="{ paused: state.paused }" />
    <span class="dur" :class="{ 'paused-text': state.paused }">
      {{ state.status === "off" ? "已关闭" : state.paused ? "已暂停" : durationText() }}
    </span>

    <span class="sep" />

    <!-- 使用者：令牌持有者 + purpose（HUD 空间有限，只显示持有者；其余见主窗） -->
    <span v-if="holderInfo" class="holder" :title="holderInfo.purpose || holderInfo.name">
      <span class="holder-name">{{ holderInfo.name }}</span>
      <span v-if="holderInfo.purpose" class="holder-purpose">{{ holderInfo.purpose }}</span>
    </span>
    <span v-else class="idle">空闲</span>

    <!-- 冲突：人类在场可视化（写 gate 已让路） -->
    <span v-if="state.human_active" class="chip chip-warning" title="检测到你在使用鼠标/键盘——agent 写操作已让路，闲置约 2 秒后自动恢复">
      <MousePointer2 :size="12" />
      <span>你在操作</span>
    </span>

    <!-- 等待：队列 + 授予队首 -->
    <button
      v-if="state.queue.length > 0"
      class="chip chip-action"
      :title="`排队：${state.queue.join('、')}——点击把操作权授予队首`"
      @click="grantQueueHead"
    >
      <span>{{ state.queue.length }} 等待</span>
      <ChevronsRight :size="12" />
    </button>

    <!-- 成本：截图张数 -->
    <span class="chip chip-plain" title="本次共享累计截图张数">
      <Camera :size="12" />
      <span>{{ state.screenshot_count }}</span>
    </span>

    <span class="flex-spacer" />

    <!-- 显示器切换 -->
    <span class="monitor">
      <button class="icon-btn" title="上一台显示器" @click="cycleMonitor(-1)">
        <ChevronLeft :size="14" />
      </button>
      <span class="monitor-label">屏 {{ state.hud_monitor + 1 }}</span>
      <button class="icon-btn" title="下一台显示器" @click="cycleMonitor(1)">
        <ChevronRight :size="14" />
      </button>
    </span>

    <span class="sep" />

    <!-- 控制：暂停/恢复 · 终止 · 收起（终止常驻，§4.7） -->
    <button class="icon-btn" :title="state.paused ? '恢复屏幕操作' : '暂停屏幕操作（读写挂起，授权保持）'" @click="togglePause">
      <Play v-if="state.paused" :size="14" class="warn" />
      <Pause v-else :size="14" />
    </button>
    <button class="icon-btn stop" title="结束屏幕共享（全部会话停止）" @click="terminate">
      <X :size="15" />
    </button>
    <button class="icon-btn" title="收起（agent 操作屏幕时也会自动收缩让路）" @click="collapsed = true">
      <ChevronsRight :size="15" />
    </button>
  </div>
</template>

<style scoped>
.hud {
  display: flex;
  align-items: center;
  height: 100vh;
  box-sizing: border-box;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: 0 4px 16px rgb(0 0 0 / 0.14);
  font-family: var(--ip-font-sans);
  color: var(--ip-color-text-primary);
  user-select: none;
}

.hud-full {
  gap: var(--ip-spacing-2);
  padding: 0 var(--ip-spacing-3);
}

.hud-mini {
  gap: var(--ip-spacing-1);
  padding: 0 var(--ip-spacing-2);
  font-size: var(--ip-text-micro-size);
}

.lead-icon {
  color: var(--ip-primary-500);
  flex-shrink: 0;
}
.lead-icon.paused { color: var(--ip-warning-base); }

.dur {
  font-size: var(--ip-text-caption-size, 12px);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.paused-text { color: var(--ip-warning-text); }

.mini-text { white-space: nowrap; }

.sep {
  width: 1px;
  height: 18px;
  background: var(--ip-color-border-default);
  flex-shrink: 0;
}

.holder {
  display: inline-flex;
  align-items: baseline;
  gap: var(--ip-spacing-1);
  min-width: 0;
  overflow: hidden;
}
.holder-name {
  font-size: var(--ip-text-caption-size, 12px);
  font-weight: 600;
  white-space: nowrap;
}
.holder-purpose {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 120px;
}

.idle {
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-tertiary);
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  padding: 2px var(--ip-spacing-2);
  border-radius: 999px;
  font-size: var(--ip-text-micro-size);
  white-space: nowrap;
  flex-shrink: 0;
}
.chip-warning {
  background: var(--ip-warning-bg);
  color: var(--ip-warning-text);
  border: 1px solid var(--ip-warning-border);
}
.chip-action {
  background: transparent;
  color: var(--ip-color-text-secondary);
  border: 1px solid var(--ip-color-border-default);
  cursor: pointer;
}
.chip-action:hover { border-color: var(--ip-color-border-strong); color: var(--ip-color-text-primary); }
.chip-plain {
  color: var(--ip-color-text-tertiary);
  border: 1px solid transparent;
}

.flex-spacer { flex: 1; }

.monitor {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}
.monitor-label {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-secondary);
  font-variant-numeric: tabular-nums;
}

.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--ip-radius-md, 6px);
  background: transparent;
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  flex-shrink: 0;
  padding: 0;
}
.icon-btn:hover {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}
.icon-btn.stop { color: var(--ip-danger-base); }
.icon-btn.stop:hover {
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
}
.icon-btn.warn { color: var(--ip-warning-base); }

.spin { animation: hud-spin 1s linear infinite; }
@keyframes hud-spin { to { transform: rotate(360deg); } }

@media (prefers-reduced-motion: reduce) {
  .spin { animation: none; }
}
</style>
