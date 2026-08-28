<script setup lang="ts">
// AuthRequestCard.vue — 工具授权内联审批卡（#10 路由模型：激活会话分支）
//
// 渲染激活会话的待处理授权请求（chat.activeConvAuthRequest），输入框上方
// 向上弹出的卡片（VSCode Claude Code 风格）——空间相邻、上下文天然清晰，
// 取代旧 ToolAuthDialog 的「全局单 modal」（后台会话的授权走 AuthNoticeStack）。
//
// #11 分层授权：允许前选范围档（仅此一次 / 此目录含子目录 / 此工具·本会话），
// 默认最小档；120s 倒计时与后端 wait_for_auth_response 的 TIMEOUT 同步，
// 到点后端自动取消并发 cancel 事件清条目（卡片随 v-if 消失）。
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import { useChatStore, TOOL_AUTH_TIMEOUT_MS } from "../../stores/chat";
import { formatJson } from "../../utils/format";
import type { AuthScope } from "../../types";

const chat = useChatStore();

const entry = computed(() => chat.activeConvAuthRequest);
const req = computed(() => entry.value?.payload ?? null);
const hasPath = computed(() => !!req.value?.file_path);

// 批次④ 步骤 1：request_screen_session 特判为二键卡——批准即开屏幕共享通道，
// scope 档（仅此一次/此目录/此工具）对它无意义（后端忽略，once 兜底无害）。
const isChannelRequest = computed(() => req.value?.tool_name === "request_screen_session");

// ---- 范围档选择（新请求到达时重置回最小档）----
const scope = ref<AuthScope>("once");
watch(
  () => req.value?.request_id,
  () => { scope.value = "once"; },
);

// ---- 120s 倒计时（250ms 粒度驱动秒数 + 进度条）----
const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | null = null;
onMounted(() => {
  timer = setInterval(() => { now.value = Date.now(); }, 250);
});
onBeforeUnmount(() => { if (timer) clearInterval(timer); });

const remainingMs = computed(() =>
  entry.value ? TOOL_AUTH_TIMEOUT_MS - (now.value - entry.value.receivedAt) : 0,
);
const expired = computed(() => remainingMs.value <= 0);
const urgent = computed(() => remainingMs.value <= 20_000);
const remainingLabel = computed(() => {
  const s = Math.max(0, Math.ceil(remainingMs.value / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
});
const progressPct = computed(() =>
  Math.max(0, Math.min(100, (remainingMs.value / TOOL_AUTH_TIMEOUT_MS) * 100)),
);

const SCOPE_OPTIONS: Array<{ value: AuthScope; label: string }> = [
  { value: "once", label: "仅此一次" },
  { value: "this_dir", label: "此目录（含子目录）" },
  { value: "this_tool", label: "此工具（本会话）" },
];
/** 无路径（Confirm 级工具）无目录可言，隐藏此目录档 */
const scopeOptions = computed(() =>
  hasPath.value ? SCOPE_OPTIONS : SCOPE_OPTIONS.filter((o) => o.value !== "this_dir"),
);

function allow() {
  if (!req.value || expired.value) return;
  void chat.respondToAuth(req.value.request_id, true, scope.value);
}
function deny() {
  if (!req.value || expired.value) return;
  void chat.respondToAuth(req.value.request_id, false);
}
</script>

<template>
  <Transition name="auth-card">
    <div v-if="req" class="auth-card" role="alertdialog" aria-label="工具授权请求">
      <!-- 倒计时进度条：随剩余时间收缩，最后 20s 转警示色 -->
      <div class="auth-progress" :class="{ urgent }">
        <div class="auth-progress-fill" :class="{ urgent }" :style="{ width: progressPct + '%' }" />
      </div>

      <div class="auth-main">
        <!-- L1 工具 + 倒计时（锁图标替代「工具授权请求」标题——上下文已在输入框上方） -->
        <div class="auth-line1">
          <svg class="auth-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>
          <span class="auth-tool-name">{{ req.tool_name }}</span>
          <span class="auth-countdown" :class="{ urgent, expired }">
            {{ expired ? "已超时" : remainingLabel }}
          </span>
        </div>

        <!-- L2 路径 + 原因（单行省略，完整内容走 title 提示）+ 参数折叠 -->
        <div class="auth-line2">
          <span v-if="hasPath" class="auth-path" :title="req.file_path">{{ req.file_path }}</span>
          <span v-if="hasPath && req.reason" class="auth-dot">·</span>
          <span v-if="req.reason" class="auth-reason" :title="req.reason">{{ req.reason }}</span>
          <details class="auth-args">
            <summary>参数</summary>
            <pre class="auth-json">{{ formatJson(req.arguments) }}</pre>
          </details>
        </div>

        <!-- L3 范围档（选择作用于「允许」）+ 拒绝/允许，一行收束。
             通道请求卡无范围概念（批准 = 开通道），隐藏单选组、按钮改语义文案 -->
        <div class="auth-line3">
          <div v-if="!isChannelRequest" class="auth-scope-options" role="radiogroup" aria-label="允许范围">
            <button
              v-for="opt in scopeOptions"
              :key="opt.value"
              type="button"
              class="auth-scope-opt"
              :class="{ active: scope === opt.value }"
              role="radio"
              :aria-checked="scope === opt.value"
              :disabled="expired"
              @click="scope = opt.value"
            >
              {{ opt.label }}
            </button>
          </div>
          <div class="auth-actions">
            <button class="auth-btn auth-btn-deny" :disabled="expired" @click="deny">拒绝</button>
            <button class="auth-btn auth-btn-allow" :disabled="expired" @click="allow">
              {{ isChannelRequest ? "开启屏幕共享" : "允许" }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.auth-card {
  position: relative;
  /* 宽度对齐输入框列（ChatInput .input-container max-width:800px + 居中）：
     审批卡悬在输入框正上方，横贯整个内容区（2K 宽屏尤甚）视觉割裂且压迫 */
  margin: 0 auto 8px;
  width: min(calc(100% - 48px), 800px);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  flex-shrink: 0;
}

.auth-progress {
  height: 3px;
  background: var(--ip-color-bg-tertiary);
  /* 卡片不裁切（参数浮层要探出主体），顶角圆角由进度条自己收 */
  border-radius: var(--ip-radius-lg) var(--ip-radius-lg) 0 0;
}
.auth-progress-fill {
  height: 100%;
  background: var(--ip-primary-500);
  transition: width 0.25s linear;
}
.auth-progress-fill.urgent { background: var(--ip-danger-base); }

/* 紧凑三行布局（手测反馈：原竖排五行太高，输入框上方压迫感强） */
.auth-main {
  padding: 8px 12px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

/* L1：锁图标 + 工具名（主信息）+ 倒计时（右缘） */
.auth-line1 {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.auth-icon { color: var(--ip-primary-600); flex-shrink: 0; }
.auth-tool-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-primary-600);
}
.auth-countdown {
  flex-shrink: 0;
  font-size: var(--ip-text-caption-size);
  font-family: var(--ip-font-mono, monospace);
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}
.auth-countdown.urgent { color: var(--ip-danger-base); }
.auth-countdown.expired { color: var(--ip-danger-base); opacity: 0.7; }

/* L2：路径 + 原因一行（超长省略，title 承载全文）+ 参数折叠（默认收起） */
.auth-line2 {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: var(--ip-text-caption-size);
}
.auth-path {
  min-width: 0;
  max-width: 45%;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-family: var(--ip-font-mono, monospace);
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
  padding: 1px 6px;
  border-radius: var(--ip-radius-sm);
}
.auth-dot { color: var(--ip-color-text-disabled); flex-shrink: 0; }
.auth-reason {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--ip-color-text-secondary);
}
.auth-args {
  flex-shrink: 0;
  margin-left: auto;
}
.auth-args[open] {
  /* 展开时脱离行流，浮在下层（不撑高卡片主体） */
  position: absolute;
  right: 12px;
  top: 34px;
  z-index: 2;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md);
  padding: 4px 8px 8px;
  max-width: 70%;
}
.auth-args summary {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
  list-style: none;
}
.auth-args summary::before { content: "▸ "; }
.auth-args[open] summary::before { content: "▾ "; }
.auth-args summary:hover { color: var(--ip-color-text-secondary); }
.auth-json {
  font-size: var(--ip-text-caption-size);
  font-family: var(--ip-font-mono, monospace);
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
  padding: 6px 8px;
  border-radius: var(--ip-radius-sm);
  max-height: 160px;
  overflow-y: auto;
  margin: 4px 0 0;
  line-height: 1.4;
}

/* L3：范围档（作用于允许）+ 拒绝/允许 */
.auth-line3 {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  min-width: 0;
}
.auth-scope-options {
  display: flex;
  gap: 4px;
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
}
.auth-scope-opt {
  flex-shrink: 0;
  padding: 2px 10px;
  border-radius: var(--ip-radius-full, 999px);
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
  color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-caption-size);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.auth-scope-opt:hover:not(:disabled) { border-color: var(--ip-primary-400); color: var(--ip-primary-600); }
.auth-scope-opt.active {
  background: var(--ip-primary-500);
  border-color: var(--ip-primary-500);
  color: #fff;
}
.auth-scope-opt:disabled { opacity: 0.5; cursor: not-allowed; }

.auth-actions { display: flex; gap: 6px; flex-shrink: 0; }
.auth-btn {
  padding: 4px 14px;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  cursor: pointer;
  border: none;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.auth-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.auth-btn-deny { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-secondary); }
.auth-btn-deny:hover:not(:disabled) { background: var(--ip-danger-bg); color: var(--ip-danger-base); }
.auth-btn-allow { background: var(--ip-primary-500); color: white; }
.auth-btn-allow:hover:not(:disabled) { opacity: 0.9; }

.auth-card-enter-active { transition: opacity 0.2s var(--ip-ease-out), transform 0.2s var(--ip-ease-out); }
.auth-card-leave-active { transition: opacity 0.15s ease-in, transform 0.15s ease-in; }
.auth-card-enter-from,
.auth-card-leave-to { opacity: 0; transform: translateY(10px); }
</style>
