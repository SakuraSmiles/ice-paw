<script setup lang="ts">
// ToolAuthDialog.vue — 工具授权确认弹窗
// 当 LLM 调用需要路径授权的工具时，由 Rust 侧 emit chat:tool-auth-request 触发。
import { useChatStore } from "../../stores/chat";

const chat = useChatStore();

function formatJson(str: string): string {
  try { return JSON.stringify(JSON.parse(str), null, 2); } catch { return str; }
}

function allow() {
  chat.respondToAuth(true);
}

function deny() {
  chat.respondToAuth(false);
}
</script>

<template>
  <Transition name="auth-overlay">
    <div v-if="chat.pendingAuthRequest" class="auth-overlay" @click.self="deny">
      <div class="auth-panel">
        <div class="auth-header">
          <svg class="auth-icon" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>
          <h3 class="auth-title">工具授权</h3>
        </div>

        <div class="auth-body">
          <div class="auth-row">
            <span class="auth-label">工具</span>
            <span class="auth-value auth-tool-name">{{ chat.pendingAuthRequest.tool_name }}</span>
          </div>
          <div class="auth-row">
            <span class="auth-label">路径</span>
            <span class="auth-value auth-path">{{ chat.pendingAuthRequest.file_path || '（无路径）' }}</span>
          </div>
          <div class="auth-row">
            <span class="auth-label">原因</span>
            <span class="auth-value auth-reason">{{ chat.pendingAuthRequest.reason }}</span>
          </div>
          <div class="auth-section">
            <div class="auth-section-label">参数</div>
            <pre class="auth-json">{{ formatJson(chat.pendingAuthRequest.arguments) }}</pre>
          </div>
        </div>

        <div class="auth-footer">
          <button class="auth-btn auth-btn-deny" @click="deny">拒绝</button>
          <button class="auth-btn auth-btn-allow" @click="allow">允许本次会话</button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.auth-overlay { position:fixed; inset:0; z-index:var(--ip-z-modal-overlay); background:rgba(0,0,0,0.35); display:flex; align-items:center; justify-content:center; backdrop-filter:blur(2px); }
.auth-panel { width:380px; max-width:90vw; background:var(--ip-color-bg-elevated); border:1px solid var(--ip-color-border-default); border-radius:var(--ip-radius-xl); box-shadow:var(--ip-shadow-xl); overflow:hidden; }
.auth-header { display:flex; align-items:center; gap:10px; padding:16px 20px 12px; }
.auth-icon { font-size:20px; line-height:1; }
.auth-title { font-size:var(--ip-text-h3-size); font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-primary); margin:0; }
.auth-body { padding:0 20px 16px; display:flex; flex-direction:column; gap:8px; }
.auth-row { display:flex; justify-content:space-between; align-items:center; gap:12px; }
.auth-label { font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-tertiary); white-space:nowrap; }
.auth-value { font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-primary); text-align:right; word-break:break-all; }
.auth-tool-name { font-weight:var(--ip-font-weight-semibold); color:var(--ip-primary-600); }
.auth-path { font-family:var(--ip-font-mono, monospace); font-size:var(--ip-text-caption-size); background:var(--ip-color-bg-tertiary); padding:2px 6px; border-radius:var(--ip-radius-sm); }
.auth-reason { color:var(--ip-color-text-secondary); }
.auth-section { margin-top:4px; }
.auth-section-label { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); margin-bottom:4px; font-weight:var(--ip-font-weight-medium); }
.auth-json { font-size:var(--ip-text-caption-size); font-family:var(--ip-font-mono, monospace); white-space:pre-wrap; word-break:break-word; color:var(--ip-color-text-secondary); background:var(--ip-color-bg-tertiary); padding:8px; border-radius:var(--ip-radius-sm); max-height:160px; overflow-y:auto; margin:0; line-height:1.4; }
.auth-footer { display:flex; gap:8px; padding:12px 20px 16px; border-top:1px solid var(--ip-color-border-default); }
.auth-btn { flex:1; padding:8px 16px; border-radius:var(--ip-radius-md); font-size:var(--ip-text-body-sm-size); font-weight:var(--ip-font-weight-medium); cursor:pointer; border:none; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.auth-btn-deny { background:var(--ip-color-bg-tertiary); color:var(--ip-color-text-secondary); }
.auth-btn-deny:hover { background:var(--ip-danger-bg); color:var(--ip-danger-base); }
.auth-btn-allow { background:var(--ip-primary-500); color:white; }
.auth-btn-allow:hover { opacity:0.9; }

.auth-overlay-enter-active { animation:auth-overlay-in 0.2s ease-out; }
.auth-overlay-leave-active { animation:auth-overlay-in 0.15s ease-in reverse; }
@keyframes auth-overlay-in { from { opacity:0; } to { opacity:1; } }
.auth-overlay-enter-active .auth-panel { animation:auth-panel-in 0.2s ease-out; }
.auth-overlay-leave-active .auth-panel { animation:auth-panel-in 0.15s ease-in reverse; }
@keyframes auth-panel-in { from { opacity:0; transform:scale(0.95) translateY(8px); } to { opacity:1; transform:scale(1) translateY(0); } }
</style>
