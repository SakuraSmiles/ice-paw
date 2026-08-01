<script setup lang="ts">
// LogSettings.vue — 运行日志（设置→日志）
// 读取磁盘日志文件末尾若干行，按级别高亮展示；支持手动刷新 / 自动刷新。
import { ref, computed, onMounted, onUnmounted, nextTick } from "vue";
import { bridge } from "../../api/bridge";
import { formatTime } from "../../utils/time";
import Switch from "../../components/common/Switch.vue";

const AUTO_INTERVAL_MS = 5000;

const rawLines = ref<string[]>([]);
const loading = ref(false);
const error = ref("");
const autoRefresh = ref(false);
let autoTimer: number | null = null;

// 滚动容器引用（自动刷到底部，最新日志在末尾）
const bodyRef = ref<HTMLElement | null>(null);

interface LogEntry {
  raw: string;
  time: string; // 本地 HH:MM:SS（解析失败为空）
  fullTime: string; // 原始时间戳（tooltip）
  level: string; // INFO/WARN/...（解析失败为空）
  target: string;
  message: string;
  parsed: boolean;
}

// tracing 默认 fmt：<RFC3339>  <LEVEL> <target>: <message>
const LOG_RE =
  /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z?)\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s+(.+?):\s?(.*)$/;

/** RFC3339(UTC) → 配置时区下 HH:MM:SS；解析失败回退原始字符串 */
function toLocalTime(fullTime: string): string {
  // JS Date 仅毫秒精度，截断微秒位避免 Invalid Date
  const norm = fullTime.replace(/\.\d+/, (m) => m.slice(0, 4));
  const d = new Date(norm);
  if (Number.isNaN(d.getTime())) return fullTime;
  return formatTime(norm, true);
}

function parseLine(line: string): LogEntry {
  const m = LOG_RE.exec(line);
  if (!m) {
    return { raw: line, time: "", fullTime: "", level: "", target: "", message: line, parsed: false };
  }
  const [, fullTime, level, target, message] = m;
  return {
    raw: line,
    time: toLocalTime(fullTime),
    fullTime,
    level,
    target,
    message,
    parsed: true,
  };
}

const entries = computed<LogEntry[]>(() => rawLines.value.map(parseLine));
const count = computed(() => rawLines.value.length);

function levelClass(level: string): string {
  switch (level) {
    case "ERROR":
      return "lvl-error";
    case "WARN":
      return "lvl-warn";
    case "INFO":
      return "lvl-info";
    case "DEBUG":
      return "lvl-debug";
    case "TRACE":
      return "lvl-trace";
    default:
      return "lvl-raw";
  }
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    rawLines.value = await bridge.logs.get();
    await nextTick();
    scrollToBottom();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
    rawLines.value = [];
  } finally {
    loading.value = false;
  }
}

function scrollToBottom() {
  const el = bodyRef.value;
  if (el) el.scrollTop = el.scrollHeight;
}

/** Switch 切换自动刷新 */
function onAutoChange(on: boolean) {
  autoRefresh.value = on;
  if (autoTimer) {
    clearInterval(autoTimer);
    autoTimer = null;
  }
  if (on) {
    autoTimer = window.setInterval(load, AUTO_INTERVAL_MS);
  }
}

onMounted(load);
onUnmounted(() => {
  if (autoTimer) clearInterval(autoTimer);
});
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">运行日志</h2>
      <div class="header-actions">
        <label class="auto-toggle" title="每 5 秒自动刷新">
          <span>自动刷新</span>
          <Switch :model-value="autoRefresh" @update:model-value="onAutoChange" />
        </label>
        <button type="button" class="refresh-btn" :disabled="loading" @click="load">
          <svg
            class="refresh-icon"
            :class="{ spinning: loading }"
            width="14" height="14" viewBox="0 0 24 24"
            fill="none" stroke="currentColor" stroke-width="2"
            stroke-linecap="round" stroke-linejoin="round"
          >
            <polyline points="23 4 23 10 17 10" />
            <polyline points="1 20 1 14 7 14" />
            <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
          </svg>
          <span>{{ loading ? "刷新中" : "刷新" }}</span>
        </button>
      </div>
    </div>

    <div class="log-area">
      <div class="log-card">
        <!-- 加载中 -->
        <div v-if="loading && !rawLines.length" class="log-state">加载中…</div>
        <!-- 错误 -->
        <div v-else-if="error" class="log-state is-error">
          <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" />
          </svg>
          <span>读取日志失败：{{ error }}</span>
          <button type="button" class="refresh-btn" @click="load">重试</button>
        </div>
        <!-- 空 -->
        <div v-else-if="!rawLines.length" class="log-state">暂无日志记录</div>
        <!-- 日志列表 -->
        <div v-else ref="bodyRef" class="log-body">
          <div
            v-for="(entry, i) in entries"
            :key="i"
            class="log-line"
            :class="[levelClass(entry.level), { 'log-continuation': !entry.parsed }]"
          >
            <span class="log-time" :title="entry.fullTime">{{ entry.time || "—" }}</span>
            <span class="log-level">{{ entry.level || "·" }}</span>
            <span class="log-target">{{ entry.target }}</span>
            <span class="log-msg">{{ entry.message }}</span>
          </div>
        </div>
      </div>

      <!-- 行数（极简：无框无底色，仅右下角淡字） -->
      <div v-if="rawLines.length" class="log-foot">{{ count }} 行</div>
    </div>
  </div>
</template>

<style scoped>
.settings-content-inner {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 0;
  min-height: 0;
}

/* ===== 头部（与 McpSettings / KbSettings 一致） ===== */
.content-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 20px 28px 0;
  flex-shrink: 0;
  height: 56px;
  gap: 12px;
}
.content-title {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

/* 自动刷新开关 */
.auto-toggle {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  user-select: none;
}

/* 刷新按钮（与 app 文字按钮风格统一） */
.refresh-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 28px;
  padding: 0 12px;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background: none;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.refresh-btn:hover:not(:disabled) {
  color: var(--ip-primary-600);
  border-color: var(--ip-primary-300);
  background-color: var(--ip-color-bg-tertiary);
}
.refresh-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.refresh-icon.spinning {
  animation: log-spin 0.9s linear infinite;
}
@keyframes log-spin {
  to { transform: rotate(360deg); }
}

/* ===== 日志区域（卡片容器，与 MCP 卡片同语言） ===== */
.log-area {
  flex: 1;
  min-height: 0;
  padding: 8px 28px 24px;
  display: flex;
  flex-direction: column;
}

.log-card {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  overflow: hidden;
}

/* ===== 日志正文 ===== */
.log-body {
  flex: 1;
  overflow-y: auto;
  padding: 8px 0;
  font-family: var(--ip-font-mono);
  font-size: 12px;
  line-height: 1.75;
}

.log-line {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 0 16px 0 14px;
  /* 左侧 2px 严重度色条（默认透明，ERROR/WARN 着色） */
  border-left: 2px solid transparent;
  color: var(--ip-color-text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.log-line:hover {
  background-color: var(--ip-color-bg-tertiary);
}

.log-time {
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}

.log-level {
  flex-shrink: 0;
  width: 44px;
  text-align: center;
  font-weight: var(--ip-font-weight-semibold);
  letter-spacing: 0.02em;
  color: var(--ip-color-text-tertiary);
}

.log-target {
  flex-shrink: 0;
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ip-color-text-tertiary);
}

.log-msg {
  flex: 1;
  min-width: 0;
  color: var(--ip-color-text-primary);
}

/* 级别配色（全部走主题自适应的语义令牌） */
.lvl-error {
  border-left-color: var(--ip-danger-base);
}
.lvl-error .log-level {
  color: var(--ip-danger-base);
}
.lvl-error .log-msg {
  color: var(--ip-danger-text);
}

.lvl-warn {
  border-left-color: var(--ip-warning-base);
}
.lvl-warn .log-level {
  color: var(--ip-warning-base);
}
.lvl-warn .log-msg {
  color: var(--ip-warning-text);
}

.lvl-info .log-level {
  color: var(--ip-primary-500);
}

.lvl-debug .log-msg,
.lvl-trace .log-msg {
  color: var(--ip-color-text-tertiary);
}

/* 结构化解析失败的续行（堆栈/多行）缩进对齐 */
.log-continuation {
  padding-left: 72px;
  border-left-color: transparent;
}
.log-continuation .log-msg {
  color: var(--ip-color-text-tertiary);
  font-style: italic;
}

/* 行数提示（极简：卡片外、无框无底色，右下角淡字） */
.log-foot {
  flex-shrink: 0;
  margin-top: 8px;
  text-align: right;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

/* ===== 状态占位 ===== */
.log-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-body-sm-size);
}
.log-state.is-error {
  flex-direction: column;
  gap: 12px;
  color: var(--ip-danger-text);
}
</style>
