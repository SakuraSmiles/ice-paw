<script setup lang="ts">
// TaskQuickEntry.vue — 侧栏定时任务快速入口（设置按钮上方）。
// 形态拍板（设计 §7）：单行常驻（最近一条：状态点 + 任务名 + 相对时）+ hover
// 展开最多 3 条；无执行记录不渲染（空状态不占侧栏）；点击进设置·定时任务。
// 数据 = 30s 轮询跨任务最近执行（task_runs 派生管理数据，无事件流——轮询是
// 轻量兜底；onUnmounted 清理定时器，W6 批纪律）。
import { onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import { Clock } from "@lucide/vue";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { bridge } from "../../api/bridge";
import { timeAgo } from "../../utils/time";
import type { RecentTaskRun } from "../../types";

const router = useRouter();
const runs = ref<RecentTaskRun[]>([]);
let timer: ReturnType<typeof setInterval> | null = null;

// ===== OS 失焦通知（0.9.3 补批）：任务完成/失败且应用不在前台时系统通知 =====
// 恰一次语义：run_id 进已通知集不再重发（容量 50 滚动防泄漏）；聚焦时静默
// （用户自己能看到侧栏入口）。权限失败静默降级（入口可见性兜底）。
let notifiedRunIds = new Set<string>();
let notifReady = false;

async function ensureNotifPermission(): Promise<void> {
  if (notifReady) return;
  try {
    if (!(await isPermissionGranted())) {
      notifReady = (await requestPermission()) === "granted";
    } else {
      notifReady = true;
    }
  } catch { notifReady = false; }
}

function maybeNotify(list: RecentTaskRun[]): void {
  if (!notifReady || !document.hidden) return;
  for (const r of list) {
    if (r.status === "running" || notifiedRunIds.has(r.run_id)) continue;
    notifiedRunIds.add(r.run_id);
    const firstLine = (r.summary ?? "").split("\n")[0].trim();
    sendNotification({
      title: r.status === "error" ? `定时任务失败：${r.task_name}` : `定时任务完成：${r.task_name}`,
      body: r.status === "error" ? (r.summary ?? "查看执行记录了解详情") : (firstLine || "点击查看结果"),
    });
  }
  // 滚动清理（容量 50：recentRuns 只取 3，50 足够跨长会话去重）
  if (notifiedRunIds.size > 50) {
    notifiedRunIds = new Set([...notifiedRunIds].slice(-50));
  }
}

async function refresh() {
  try {
    const list = await bridge.tasks.recentRuns(3);
    runs.value = list;
    maybeNotify(list);
  } catch { /* 下轮再试——快速入口是非关键路径，不弹错误 */ }
}

onMounted(() => {
  ensureNotifPermission();
  refresh();
  timer = setInterval(refresh, 30_000);
});
onUnmounted(() => { if (timer) clearInterval(timer); });

const statusTitle = (s: string) =>
  ({ done: "已完成", running: "运行中", error: "失败", missed: "已错过" }[s] ?? s);
</script>

<template>
  <div v-if="runs.length" class="task-entry" title="定时任务">
    <!-- 单行常驻（最近一条） -->
    <button class="entry-line" @click="router.push('/settings/tasks')">
      <span :class="['dot', runs[0]!.status]" :title="statusTitle(runs[0]!.status)" />
      <Clock :size="12" class="entry-icon" />
      <span class="entry-name">{{ runs[0]!.task_name }}</span>
      <span class="entry-time">{{ timeAgo(runs[0]!.started_at) }}</span>
    </button>
    <!-- hover 展开面板（最多 3 条） -->
    <div class="entry-pop">
      <button
        v-for="r in runs" :key="r.run_id"
        class="pop-row" @click="router.push('/settings/tasks')"
      >
        <span :class="['dot', r.status]" :title="statusTitle(r.status)" />
        <span class="pop-name">{{ r.task_name }}</span>
        <span class="pop-status">{{ statusTitle(r.status) }}</span>
        <span class="pop-time">{{ timeAgo(r.started_at) }}</span>
      </button>
      <button class="pop-manage" @click="router.push('/settings/tasks')">管理定时任务…</button>
    </div>
  </div>
</template>

<style scoped>
.task-entry { position: relative; margin: 0 var(--ip-spacing-3) var(--ip-spacing-1); flex-shrink: 0; }

/* 入口行：侧栏原生件风格（朴素行 + hover 微亮，footer-btn 同视觉重量） */
.entry-line { display: flex; align-items: center; gap: 6px; width: 100%; padding: var(--ip-spacing-1_5) var(--ip-spacing-2_5); border: none; border-radius: var(--ip-radius-md); background: transparent; color: var(--ip-color-text-secondary); font: inherit; font-size: var(--ip-text-micro-size); cursor: pointer; transition: background-color var(--ip-duration-fast) var(--ip-ease-out), color var(--ip-duration-fast) var(--ip-ease-out); }
.entry-line:hover { background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }
.entry-line:hover + .entry-pop, .entry-pop:hover { opacity: 1; visibility: visible; transform: translateY(0); }

.entry-icon { flex-shrink: 0; color: var(--ip-color-icon-muted); }
.entry-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ip-color-text-primary); }
.entry-time { flex-shrink: 0; color: var(--ip-color-text-disabled); }

.dot { flex-shrink: 0; width: 6px; height: 6px; border-radius: 50%; }
.dot.done { background: var(--ip-success-base, #107757); }
.dot.running { background: var(--ip-primary-500); }
.dot.error { background: var(--ip-danger-base); }
.dot.missed { background: var(--ip-warning-base, #926c12); }

/* hover 面板：绝对定位于入口上方（footer 区在底部），与收起态 flyout 同 z 档 */
.entry-pop { position: absolute; bottom: calc(100% + 4px); left: 0; right: 0; z-index: var(--ip-z-dropdown, 100); display: flex; flex-direction: column; padding: var(--ip-spacing-1_5); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); background: var(--ip-color-bg-secondary); box-shadow: 0 6px 20px rgba(0, 0, 0, 0.14); opacity: 0; visibility: hidden; transform: translateY(4px); transition: opacity var(--ip-duration-fast) var(--ip-ease-out), transform var(--ip-duration-fast) var(--ip-ease-out), visibility var(--ip-duration-fast); }

.pop-row { display: flex; align-items: center; gap: 6px; padding: var(--ip-spacing-1_5) var(--ip-spacing-2); border: none; border-radius: var(--ip-radius-sm); background: transparent; color: var(--ip-color-text-secondary); font: inherit; font-size: var(--ip-text-micro-size); cursor: pointer; text-align: left; }
.pop-row:hover { background: var(--ip-color-bg-tertiary); }
.pop-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ip-color-text-primary); }
.pop-status { flex-shrink: 0; color: var(--ip-color-text-disabled); }
.pop-time { flex-shrink: 0; color: var(--ip-color-text-disabled); }
.pop-manage { margin-top: 2px; padding: var(--ip-spacing-1_5) var(--ip-spacing-2); border: none; border-top: 1px solid var(--ip-color-border-default); background: transparent; color: var(--ip-primary-600); font: inherit; font-size: var(--ip-text-micro-size); cursor: pointer; text-align: left; }
.pop-manage:hover { color: var(--ip-primary-700, var(--ip-primary-600)); }
</style>
