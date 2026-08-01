<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted } from "vue";
import { useChatStore }from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import { bridge } from "../../api/bridge";

const chat = useChatStore();
const agent = useAgentStore();

const editing = ref(false);
const editValue = ref("");
const editInput = ref<HTMLInputElement | null>(null);

const confirming = ref(false);
const menuOpen = ref(false);
const menuRef = ref<HTMLElement | null>(null);

function onDocClick(e: MouseEvent) {
  if (menuOpen.value && menuRef.value && !menuRef.value.contains(e.target as Node)) {
    closeMenu();
  }
}

onMounted(() => document.addEventListener("click", onDocClick));
onUnmounted(() => document.removeEventListener("click", onDocClick));

const activeAgent = computed(() => {
  const conv = chat.activeConversation;
  if (!conv) return null;
  return agent.getById(conv.agent_id);
});

function startEdit() {
  const conv = chat.activeConversation;
  if (!conv) return;
  editValue.value = conv.title || "";
  editing.value = true;
  nextTick(() => editInput.value?.focus());
}

async function saveEdit() {
  const conv = chat.activeConversation;
  if (!conv) return;
  editing.value = false;
  const newTitle = editValue.value.trim();
  if (newTitle && newTitle !== (conv.title || "")) {
    try {
      await bridge.conversations.rename(conv.id, newTitle);
      conv.title = newTitle;
    } catch (e) {
      // 重命名失败：标题不改（保持旧值），仅记录日志，避免 unhandled rejection
      console.error("重命名会话失败:", e);
    }
  }
}

function cancelEdit() { editing.value = false; }

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Enter") saveEdit();
  if (e.key === "Escape") cancelEdit();
}

function startDelete() { confirming.value = true; }

function cancelDelete() { confirming.value = false; }

async function confirmDelete() {
  const conv = chat.activeConversation;
  if (!conv) return;
  confirming.value = false;
  menuOpen.value = false;
  await chat.deleteConversation(conv.id);
}

function toggleMenu() { menuOpen.value = !menuOpen.value; if (!menuOpen.value) confirming.value = false; }

function closeMenu() { menuOpen.value = false; confirming.value = false; }

async function togglePin() {
  const conv = chat.activeConversation;
  if (!conv) return;
  await chat.pinConversation(conv.id, !conv.pinned);
  menuOpen.value = false;
}

const showInfo = ref(false);

function viewInfo() {
  menuOpen.value = false;
  showInfo.value = true;
}
</script>

<template>
  <header class="chat-header">
    <div class="header-left">
      <div class="header-info">
        <input
          v-if="editing"
          ref="editInput"
          v-model="editValue"
          class="header-edit-input"
          @keydown="handleKeydown"
          @blur="saveEdit"
          @click.stop
        />
        <h1 v-else class="header-title" @dblclick="startEdit">
          <span class="header-title-text">{{ chat.activeConversation?.title || "新对话" }}</span>
        </h1>
        <div class="header-meta">
          <span v-if="activeAgent" class="header-agent">{{ activeAgent.name }}</span>
          <span v-if="activeAgent" class="header-sep">·</span>
          <span v-if="activeAgent" class="header-model">{{ activeAgent.model }}</span>
          <span v-else class="header-hint">选择一个对话开始</span>
        </div>
      </div>
    </div>
    <div class="header-right">
      <div v-if="chat.activeConversation" ref="menuRef" class="menu-wrapper">
        <button class="header-btn" title="更多" @click.stop="toggleMenu">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /><circle cx="5" cy="12" r="1" /></svg>
        </button>
        <Transition name="dropdown">
          <div v-if="menuOpen" class="dropdown-menu" @click.stop>
          <button class="dropdown-item" @click="togglePin">
            <!-- 已置顶：填充实心图标 → 取消置顶 -->
            <svg v-if="chat.activeConversation?.pinned" width="14" height="14" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2z" /></svg>
            <!-- 未置顶：描边空心图标 → 置顶 -->
            <svg v-else width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2z" /></svg>
            <span>{{ chat.activeConversation?.pinned ? "取消置顶" : "置顶" }}</span>
          </button>
          <button class="dropdown-item" @click="viewInfo">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="12" y1="16" x2="12" y2="12" /><line x1="12" y1="8" x2="12.01" y2="8" /></svg>
            <span>查看信息</span>
          </button>
          <div class="dropdown-divider"></div>
          <div v-if="confirming" class="menu-confirm">
            <span class="menu-confirm-text">确认删除？</span>
            <div class="menu-confirm-actions">
              <button class="menu-confirm-yes" @click="confirmDelete">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              </button>
              <button class="menu-confirm-no" @click="cancelDelete">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
              </button>
            </div>
          </div>
          <button v-else class="dropdown-item dropdown-danger" @click="startDelete">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg>
            <span>删除对话</span>
          </button>
        </div></Transition>
      </div>
    </div>
  </header>

  <Transition name="overlay">
    <div v-if="showInfo" class="info-overlay" @click.self="showInfo = false">
      <div class="info-panel">
        <div class="info-header">
          <h3 class="info-title">对话信息</h3>
          <button class="info-close" @click="showInfo = false">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
          </button>
        </div>
        <div class="info-body">
          <div class="info-row"><span class="info-label">标题</span><span class="info-value">{{ chat.activeConversation?.title || "未命名" }}</span></div>
          <div class="info-row"><span class="info-label">助手</span><span class="info-value">{{ activeAgent?.name || "未知" }}</span></div>
          <div class="info-row"><span class="info-label">模型</span><span class="info-value">{{ activeAgent?.model || "未知" }}</span></div>
          <div class="info-row"><span class="info-label">置顶</span><span class="info-value">{{ chat.activeConversation?.pinned ? "是" : "否" }}</span></div>
          <div class="info-row"><span class="info-label">消息数</span><span class="info-value">{{ chat.messages.length }}</span></div>
          <div class="info-row"><span class="info-label">创建时间</span><span class="info-value">{{ chat.activeConversation?.created_at || "未知" }}</span></div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.chat-header { display:flex; align-items:center; justify-content:space-between; padding:14px 24px; min-height:68px; border-bottom:1px solid var(--color-chat-header-border); background-color:var(--color-chat-header-bg); backdrop-filter:blur(8px); flex-shrink:0; position:relative; z-index:1; }
.header-left { display:flex; align-items:center; }
.header-info { display:flex; flex-direction:column; gap:2px; }
.header-title { font-size:var(--ip-text-body-size); font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-primary); margin:0; line-height:1.4; cursor:default; }
.header-title-text { padding-bottom:1px; border-bottom:1px solid transparent; transition:border-color var(--ip-duration-fast) var(--ip-ease-out); }
.header-title:hover .header-title-text { border-bottom-color:var(--ip-color-text-tertiary); }
.header-edit-input { font-size:var(--ip-text-body-size); font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-primary); background:var(--color-input-bg); border:1px solid var(--color-input-focus-border); border-radius:var(--ip-radius-md); padding:2px 8px; outline:none; width:100%; min-width:200px; font-family:inherit; box-shadow:0 0 0 3px rgba(46,141,100,0.12); }
.header-meta { display:flex; align-items:center; gap:6px; }
.header-agent { font-size:var(--ip-text-caption-size); color:var(--ip-primary-600); line-height:1.4; font-weight:var(--ip-font-weight-medium); }
.header-sep { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); line-height:1.4; }
.header-model { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); line-height:1.4; }
.header-hint { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); line-height:1.4; }
.header-right { display:flex; align-items:center; gap:4px; }
.header-btn { display:flex; align-items:center; justify-content:center; width:32px; height:32px; border-radius:var(--ip-radius-md); color:var(--ip-color-text-secondary); border:none; cursor:pointer; background:transparent; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.header-btn:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-primary); }

.menu-wrapper { position:relative; }
.dropdown-menu { position:absolute; top:calc(100% + 4px); right:0; z-index:100; min-width:180px; background:var(--ip-color-bg-elevated); border:1px solid var(--ip-color-border-default); border-radius:var(--ip-radius-lg); box-shadow:var(--ip-shadow-lg); padding:4px; display:flex; flex-direction:column; gap:2px; }
.dropdown-item { display:flex; align-items:center; gap:8px; width:100%; padding:8px 12px; border:none; border-radius:var(--ip-radius-md); background:transparent; cursor:pointer; font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-secondary); transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.dropdown-item:hover { background:var(--ip-color-bg-tertiary); color:var(--ip-color-text-primary); }
.dropdown-divider { height:1px; background:var(--ip-color-border-default); margin:2px 8px; }

.menu-confirm { display:flex; align-items:center; justify-content:space-between; padding:8px 12px; gap:8px; }
.menu-confirm-text { font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-secondary); white-space:nowrap; }
.menu-confirm-actions { display:flex; gap:4px; }
.menu-confirm-yes, .menu-confirm-no { display:flex; align-items:center; justify-content:center; width:28px; height:28px; border:none; border-radius:var(--ip-radius-md); cursor:pointer; background:transparent; }
.menu-confirm-yes:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-primary); }
.menu-confirm-no:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-primary); }

.info-overlay { position:fixed; inset:0; z-index:var(--ip-z-modal-overlay); background:rgba(0,0,0,0.3); display:flex; align-items:center; justify-content:center; }
.info-panel { width:340px; background:var(--ip-color-bg-elevated); border:1px solid var(--ip-color-border-default); border-radius:var(--ip-radius-xl); box-shadow:var(--ip-shadow-xl); overflow:hidden; }
.info-header { display:flex; align-items:center; justify-content:space-between; padding:16px 20px 12px; }
.info-title { font-size:var(--ip-text-h3-size); font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-primary); margin:0; }
.info-close { display:flex; align-items:center; justify-content:center; width:28px; height:28px; border-radius:var(--ip-radius-md); cursor:pointer; color:var(--ip-color-text-secondary); background:none; border:none; }
.info-close:hover { background:var(--ip-color-bg-tertiary); color:var(--ip-color-text-primary); }
.info-body { padding:0 20px 20px; display:flex; flex-direction:column; gap:10px; }
.info-row { display:flex; justify-content:space-between; align-items:center; }
.info-label { font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-tertiary); }
.info-value { font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-primary); font-weight:var(--ip-font-weight-medium); }

/* ===== 下拉菜单动画 ===== */
.dropdown-enter-active { animation:drop-in 0.15s ease-out; }
.dropdown-leave-active { animation:drop-in 0.1s ease-in reverse; }
@keyframes drop-in {
  from { opacity:0; transform:translateY(-4px) scale(0.96); }
  to { opacity:1; transform:translateY(0) scale(1); }
}

/* ===== 信息弹窗动画 ===== */
.overlay-enter-active { animation:overlay-in 0.2s ease-out; }
.overlay-leave-active { animation:overlay-in 0.15s ease-in reverse; }
@keyframes overlay-in {
  from { opacity:0; }
  to { opacity:1; }
}
.overlay-enter-active .info-panel { animation:panel-in 0.2s ease-out; }
.overlay-leave-active .info-panel { animation:panel-in 0.15s ease-in reverse; }
@keyframes panel-in {
  from { opacity:0; transform:scale(0.95) translateY(8px); }
  to { opacity:1; transform:scale(1) translateY(0); }
}
</style>
