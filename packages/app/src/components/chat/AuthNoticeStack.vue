<script setup lang="ts">
// AuthNoticeStack.vue — 后台会话授权通知栈（#10 路由模型：后台分支）
//
// 渲染「非激活会话」的待处理授权请求（chat.backgroundAuthRequests）——
// 用户在别的会话/别的页面时，右下角全局通知栈弹出，每条带**会话身份**
// （会话标题 + 工具/路径摘要），可直接允许（本次）/拒绝，或点击跳回
// 该会话用内联卡选完整范围档。取代旧 ToolAuthDialog 的「后台也全局弹
// modal」混淆源。挂载在 AppLayout（全局，所有页面可见）。
import { ref, onMounted, onBeforeUnmount } from "vue";
import { useRouter } from "vue-router";
import { useChatStore, TOOL_AUTH_TIMEOUT_MS } from "../../stores/chat";
import type {
  DelegationAuthRequestPayload,
  ToolAuthRequestPayload,
} from "../../types";

const chat = useChatStore();
const router = useRouter();

/** 会话身份：标题查会话列表（无则兜底文案）*/
function convTitle(convId: string): string {
  return chat.conversations.find((c) => c.id === convId)?.title ?? "后台会话";
}

// ---- 120s 倒计时（单一定时器驱动全部通知的秒数显示）----
const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | null = null;
onMounted(() => {
  timer = setInterval(() => { now.value = Date.now(); }, 500);
});
onBeforeUnmount(() => { if (timer) clearInterval(timer); });

function remainingLabel(receivedAt: number): string {
  const s = Math.max(0, Math.ceil((TOOL_AUTH_TIMEOUT_MS - (now.value - receivedAt)) / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}
function urgent(receivedAt: number): boolean {
  return TOOL_AUTH_TIMEOUT_MS - (now.value - receivedAt) <= 20_000;
}

function allowOnce(requestId: string) {
  // 通知上不猜意图：委派授权的「允许」= 逐次审批档（结构化预授权档选择
  // 只在应用内卡片——与系统 toast 批准按钮同语义）
  void chat.respondToAuth(requestId, true, "once");
}
function deny(requestId: string) {
  void chat.respondToAuth(requestId, false);
}
/** 点击通知主体 → 跳回该会话（切 activeConv 后请求转入内联卡，可选完整档）*/
function jumpTo(convId: string) {
  void router.push({ name: "Home" });
  chat.selectConversation(convId);
}
/** union 判别（类型守卫，模板 v-if 窄化用）：payload 带 agent_name = 委派授权 */
function isDelegation(
  payload: ToolAuthRequestPayload | DelegationAuthRequestPayload,
): payload is DelegationAuthRequestPayload {
  return "agent_name" in payload;
}
</script>

<template>
  <div v-if="chat.backgroundAuthRequests.length > 0" class="auth-notice-stack" aria-live="polite">
    <TransitionGroup name="auth-notice">
      <div
        v-for="[convId, entry] in chat.backgroundAuthRequests"
        :key="entry.payload.request_id"
        class="auth-notice"
        role="alert"
      >
        <div class="notice-head">
          <svg class="notice-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>
          <span class="notice-conv" :title="convTitle(convId)">{{ convTitle(convId) }}</span>
          <span class="notice-countdown" :class="{ urgent: urgent(entry.receivedAt) }">
            {{ remainingLabel(entry.receivedAt) }}
          </span>
        </div>
        <button class="notice-body" type="button" @click="jumpTo(convId)">
          <!-- 委派授权条目（agent_name 判别）标题带目标 agent；工具授权显示工具名 -->
          <div v-if="isDelegation(entry.payload)" class="notice-tool">
            委派给 {{ entry.payload.agent_name }}
          </div>
          <div v-else class="notice-tool">{{ entry.payload.tool_name }} 请求授权</div>
          <div
            v-if="!isDelegation(entry.payload) && entry.payload.file_path"
            class="notice-path"
          >{{ entry.payload.file_path }}</div>
          <div class="notice-hint">点击查看详情 · 跳转会话</div>
        </button>
        <div class="notice-actions">
          <button class="notice-btn notice-btn-deny" type="button" @click="deny(entry.payload.request_id)">拒绝</button>
          <button class="notice-btn notice-btn-allow" type="button" @click="allowOnce(entry.payload.request_id)">允许（本次）</button>
        </div>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.auth-notice-stack {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: var(--ip-z-notification);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2_5);
  width: 300px;
  max-width: calc(100vw - 32px);
  pointer-events: none; /* 容器不挡下层交互，条目自身恢复 */
}
.auth-notice {
  pointer-events: auto;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  overflow: hidden;
}

.notice-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px 6px;
}
.notice-icon { color: var(--ip-primary-600); flex-shrink: 0; }
.notice-conv {
  flex: 1;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.notice-countdown {
  font-size: var(--ip-text-caption-size);
  font-family: var(--ip-font-mono, monospace);
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}
.notice-countdown.urgent { color: var(--ip-danger-base); }

.notice-body {
  display: block;
  width: 100%;
  text-align: left;
  border: none;
  background: none;
  cursor: pointer;
  padding: 0 12px 8px;
  font: inherit;
  color: inherit;
}
.notice-body:hover .notice-hint { color: var(--ip-primary-600); }
.notice-tool {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  font-weight: var(--ip-font-weight-medium);
}
.notice-path {
  font-size: var(--ip-text-caption-size);
  font-family: var(--ip-font-mono, monospace);
  color: var(--ip-color-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  margin-top: 2px;
}
.notice-hint {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  margin-top: 4px;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}

.notice-actions {
  display: flex;
  gap: 6px;
  padding: 0 12px 10px;
}
.notice-btn {
  flex: 1;
  padding: 5px 10px;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  cursor: pointer;
  border: none;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.notice-btn-deny { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-secondary); }
.notice-btn-deny:hover { background: var(--ip-danger-bg); color: var(--ip-danger-base); }
.notice-btn-allow { background: var(--ip-primary-500); color: white; }
.notice-btn-allow:hover { opacity: 0.9; }

.auth-notice-enter-active { transition: opacity 0.22s var(--ip-ease-out), transform 0.22s var(--ip-ease-out); }
.auth-notice-leave-active { transition: opacity 0.16s ease-in, transform 0.16s ease-in; position: absolute; width: 100%; }
.auth-notice-enter-from { opacity: 0; transform: translateX(24px); }
.auth-notice-leave-to { opacity: 0; transform: translateX(24px); }
.auth-notice-move { transition: transform 0.22s var(--ip-ease-out); }
</style>
