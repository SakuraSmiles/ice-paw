<!--
  ChatHeader — 聊天顶部栏：会话标题编辑 + Agent 信息 + 外置操作（UX #9）

  行为：
  - 双击标题进入编辑模式（Enter 保存 / Escape 取消）
  - 外置横排操作（取代旧「更多」下拉菜单）：星标置顶（左）+ 删除（右）
  - 删除确认 = 从删除按钮向左横向扩展的确认条（非弹窗，Esc/点击外部收起）
  - 显示当前 Agent 名称 + 模型

  Props: 无（通过 chat/agent store 读取）
  Emits: 无
-->
<script setup lang="ts">
import { ref, computed, watch, nextTick, onUnmounted } from "vue";
import { useChatStore }from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import { bridge } from "../../api/bridge";

// hasTabbar：标题下方有标签条（会话态）→ 去掉底边线，与标签条视觉连成一体
//（ChatPage 传入；欢迎态无标签条，保留分割线区分标题与欢迎内容）
defineProps<{ hasTabbar?: boolean }>();

const chat = useChatStore();
const agent = useAgentStore();

const editing = ref(false);
const editValue = ref("");
const editInput = ref<HTMLInputElement | null>(null);

// ===== 删除确认条（UX #9：右锚定向左扩展，取代旧菜单内嵌确认）=====
const confirming = ref(false);
const deleteZoneRef = ref<HTMLElement | null>(null);

function onDocClick(e: MouseEvent) {
  if (confirming.value && deleteZoneRef.value && !deleteZoneRef.value.contains(e.target as Node)) {
    confirming.value = false;
  }
}
function onDocKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") confirming.value = false;
}

// U18: 只在确认条展开时注册监听，避免全局常驻
watch(confirming, (open) => {
  if (open) {
    document.addEventListener("click", onDocClick);
    document.addEventListener("keydown", onDocKeydown);
  } else {
    document.removeEventListener("click", onDocClick);
    document.removeEventListener("keydown", onDocKeydown);
  }
});
onUnmounted(() => {
  document.removeEventListener("click", onDocClick);
  document.removeEventListener("keydown", onDocKeydown);
});

const activeAgent = computed(() => {
  const conv = chat.activeConversation;
  if (!conv) return null;
  return agent.getById(conv.agent_id);
});

// ===== MA-1 任务详情 v1：委派子会话的回路与状态 =====
// 子会话不在侧栏，用户从委派卡片/任务胶囊进来——头部必须给「我是谁的任务、
// 从哪来、回哪去」。深度=1 护栏保证父会话必为 kind='chat'（在侧栏列表内）。
// 状态只有 进行中（streamingConvIds）/已结束 两态——done/failed 精确终态是
// MA-2 台账（turn_ended 派生状态机）的事，此处不伪造。
const delegation = computed(() => {
  const conv = chat.activeConversation;
  if (!conv || conv.kind !== "delegation") return null;
  const parentId = conv.parent_conversation_id ?? null;
  const parentTitle = parentId
    ? (chat.conversations.find((c) => c.id === parentId)?.title ?? "父会话")
    : null;
  return { parentId, parentTitle, running: chat.streamingConvIds.has(conv.id) };
});

function goBackToParent() {
  if (delegation.value?.parentId) chat.selectConversation(delegation.value.parentId);
}

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
  // U13: 拒绝空标题或纯空白标题，恢复旧值
  if (!newTitle) {
    editValue.value = conv.title || "";
    return;
  }
  if (newTitle !== (conv.title || "")) {
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

const deletedId = ref<string | null>(null);

async function confirmDelete() {
  const conv = chat.activeConversation;
  if (!conv) return;
  confirming.value = false;
  await chat.deleteConversation(conv.id);
  // 显示撤销 toast（5 秒后自动消失）
  deletedId.value = conv.id;
  setTimeout(() => { deletedId.value = null; }, 5000);
}

function undoDelete() {
  if (deletedId.value) {
    chat.undoDeleteConversation(deletedId.value);
    deletedId.value = null;
  }
}

async function togglePin() {
  const conv = chat.activeConversation;
  if (!conv) return;
  await chat.pinConversation(conv.id, !conv.pinned);
}
</script>

<template>
  <header class="chat-header" :class="{ 'has-tabbar': hasTabbar }">
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
          <!-- MA-1：委派子会话的回路——面包屑式上文（父会话 › 本任务）。
               父会话即返回入口（点击回父会话），比独立返回按钮更贴合
               「这是从哪来的」的导航语义，也不与标题抢视觉 -->
          <button
            v-if="delegation?.parentId"
            class="crumb-parent"
            :title="`返回父会话：${delegation.parentTitle}`"
            @click.stop="goBackToParent"
            @dblclick.stop
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6" /></svg>
            <span class="crumb-label">{{ delegation.parentTitle }}</span>
          </button>
          <span v-if="delegation?.parentId" class="crumb-sep">/</span>
          <span class="header-title-text">{{ chat.activeConversation?.title || "新对话" }}</span>
          <!-- MA-1 任务详情 v1：徽章升级为「委派任务」+ 状态点（进行中脉冲/已结束中性；
               done/failed 精确终态是 MA-2 台账，不伪造） -->
          <span
            v-if="delegation"
            class="header-kind-badge"
            :title="delegation.running ? 'agent 委派的任务 · 执行中' : 'agent 委派的任务 · 已结束'"
          >
            <span class="hdr-dot" :class="{ running: delegation.running }" />
            委派任务
          </span>
        </h1>
        <div class="header-meta">
          <span v-if="activeAgent" class="header-agent">{{ activeAgent.name }}</span>
          <span v-if="activeAgent" class="header-sep">·</span>
          <span v-if="activeAgent" class="header-model">{{ activeAgent.model }}</span>
          <span v-else class="header-hint">选择一个对话开始</span>
        </div>
      </div>
    </div>
    <!-- 外置操作（UX #9）：星标（左）+ 删除（右，占原「更多」位置）。
         删除确认 = 右锚定、向左横向扩展的确认条（覆盖星标，布局零位移） -->
    <div v-if="chat.activeConversation" class="header-right">
      <button
        class="header-btn pin-btn"
        :class="{ 'pin-hidden': confirming, pinned: chat.activeConversation?.pinned }"
        :title="chat.activeConversation?.pinned ? '取消置顶' : '置顶'"
        @click="togglePin"
      >
        <!-- 已置顶：填充实心图标 → 取消置顶 -->
        <svg v-if="chat.activeConversation?.pinned" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2z" /></svg>
        <!-- 未置顶：描边空心图标 → 置顶 -->
        <svg v-else width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2z" /></svg>
      </button>

      <div ref="deleteZoneRef" class="delete-zone">
        <Transition name="confirmbar">
          <div v-if="confirming" class="confirm-bar" @click.stop>
            <span class="confirm-text">删除此对话？</span>
            <button class="confirm-btn" @click="cancelDelete">取消</button>
            <button class="confirm-btn confirm-btn-danger" @click="confirmDelete">删除</button>
          </div>
        </Transition>
        <button v-if="!confirming" class="header-btn" title="删除对话" @click.stop="startDelete">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg>
        </button>
      </div>
    </div>
  </header>

  <!-- 删除撤销 toast -->
  <Transition name="overlay">
    <div v-if="deletedId" class="undo-toast">
      <span class="undo-toast-text">对话已删除</span>
      <button class="undo-toast-btn" @click="undoDelete">撤销</button>
    </div>
  </Transition>
</template>

<style scoped>
.chat-header { display:flex; align-items:center; justify-content:space-between; padding:14px 24px; min-height:68px; border-bottom:1px solid var(--color-chat-header-border); background-color:var(--color-chat-header-bg); backdrop-filter:blur(8px); flex-shrink:0; position:relative; z-index:1; }
/* 标签条在场（会话态）：去底边线，标题与标签条视觉一体（同底色无分割） */
.chat-header.has-tabbar { border-bottom:none; }
.header-left { display:flex; align-items:center; gap:10px; min-width:0; }
.header-info { display:flex; flex-direction:column; gap:2px; min-width:0; }
/* 面包屑上文（父会话 › 本任务）：父会话即返回入口（子会话不在侧栏列表）。
   视觉降一档（caption/tertiary），hover 提亮为 primary 强调可点；不抢标题主体 */
.crumb-parent {
  display:inline-flex; align-items:center; gap:2px; flex-shrink:0;
  max-width:220px; padding:2px 4px 2px 0; border:none; cursor:pointer;
  background:transparent; border-radius:var(--ip-radius-sm);
  color:var(--ip-color-text-tertiary); font-size:var(--ip-text-caption-size);
  font-weight:var(--ip-font-weight-regular); vertical-align:1px;
  transition:color var(--ip-duration-fast) var(--ip-ease-out);
}
.crumb-parent:hover { color:var(--ip-primary-600); }
.crumb-parent svg { flex-shrink:0; }
.crumb-label { overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
.crumb-sep { margin:0 6px 0 2px; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); vertical-align:1px; }
/* 任务状态点：与 DelegationCard/任务胶囊同语义（进行中脉冲=warning，结束=中性） */
.hdr-dot { width:7px; height:7px; border-radius:50%; flex-shrink:0; display:inline-block; margin-right:2px; background:var(--ip-color-text-tertiary); }
.hdr-dot.running { background:var(--ip-warning-base, #d97706); animation:hdr-pulse 1.2s ease-in-out infinite; }
@keyframes hdr-pulse { 0%, 100% { opacity:1; } 50% { opacity:0.35; } }
.header-title { font-size:var(--ip-text-body-size); font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-primary); margin:0; line-height:1.4; cursor:default; }
.header-kind-badge { margin-left:8px; font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-primary-600); background:var(--ip-primary-soft-bg, rgba(46,141,100,0.08)); border:1px solid var(--ip-primary-soft-border, rgba(46,141,100,0.25)); border-radius:var(--ip-radius-full, 999px); padding:1px 8px; vertical-align:1px; }
.header-title-text { padding-bottom:1px; border-bottom:1px solid transparent; transition:border-color var(--ip-duration-fast) var(--ip-ease-out); }
.header-title:hover .header-title-text { border-bottom-color:var(--ip-color-text-tertiary); }
.header-edit-input { font-size:var(--ip-text-body-size); font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-primary); background:var(--color-input-bg); border:1px solid var(--color-input-focus-border); border-radius:var(--ip-radius-md); padding:2px 8px; outline:none; width:100%; min-width:200px; font-family:inherit; box-shadow:0 0 0 3px rgba(46,141,100,0.12); }
.header-meta { display:flex; align-items:center; gap:6px; }
.header-agent { font-size:var(--ip-text-caption-size); color:var(--ip-primary-600); line-height:1.4; font-weight:var(--ip-font-weight-medium); }
.header-sep { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); line-height:1.4; }
.header-model { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); line-height:1.4; }
.header-hint { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); line-height:1.4; }
.header-right { display:flex; align-items:center; gap:4px; position:relative; }
.header-btn { display:flex; align-items:center; justify-content:center; width:32px; height:32px; border-radius:var(--ip-radius-md); color:var(--ip-color-text-secondary); border:none; cursor:pointer; background:transparent; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.header-btn:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-primary); }

/* ===== 外置星标（UX #9）：确认条展开时淡出让位（布局不动，条覆盖其上） ===== */
.pin-btn { transition:opacity var(--ip-duration-fast) var(--ip-ease-out), background-color var(--ip-duration-fast) var(--ip-ease-out), color var(--ip-duration-fast) var(--ip-ease-out); }
.pin-btn.pin-hidden { opacity:0; pointer-events:none; }
.pin-btn svg { color:var(--ip-color-text-tertiary); }
.pin-btn:hover svg { color:var(--ip-color-text-primary); }
/* 已置顶：实心星常显主色（状态可见，不只是 hover 态） */
.pin-btn.pinned svg { color:var(--ip-primary-500); }

/* ===== 删除确认条（UX #9）：右锚定、向左横向扩展 ===== */
.delete-zone { position:relative; display:flex; align-items:center; }
.confirm-bar {
  position:absolute; right:0; top:50%; transform:translateY(-50%);
  display:flex; align-items:center; gap:8px;
  max-width:260px; overflow:hidden; white-space:nowrap;
  padding:4px 6px 4px 14px;
  background:var(--ip-danger-bg, rgba(220,38,38,0.08));
  border:1px solid var(--ip-danger-border, rgba(220,38,38,0.3));
  border-radius:var(--ip-radius-md);
  z-index:2;
}
.confirm-text { font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-secondary); }
.confirm-btn {
  flex-shrink:0; padding:4px 12px; border:none; border-radius:var(--ip-radius-sm);
  font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium);
  cursor:pointer; background:transparent; color:var(--ip-color-text-secondary);
  transition:all var(--ip-duration-fast) var(--ip-ease-out);
}
.confirm-btn:hover { background:var(--ip-color-bg-tertiary); color:var(--ip-color-text-primary); }
.confirm-btn-danger { background:var(--ip-danger-base, #dc2626); color:#fff; }
.confirm-btn-danger:hover { background:var(--ip-danger-base, #dc2626); color:#fff; opacity:0.88; }

/* 宽度扩展动画：max-width 0→260 渐进放开（内容自然宽度小于上限，
   视觉上平滑长到内容宽即停），叠加淡入与微量左移 */
.confirmbar-enter-active { transition:max-width 0.22s var(--ip-ease-out), opacity 0.18s var(--ip-ease-out); }
.confirmbar-leave-active { transition:max-width 0.15s ease-in, opacity 0.12s ease-in; }
.confirmbar-enter-from, .confirmbar-leave-to { max-width:32px; opacity:0; }

/* 删除撤销 toast */
.undo-toast {
  display: flex; align-items: center; gap: 12px;
  padding: 8px 16px; margin: 0 24px;
  background: var(--ip-color-bg-elevated, #1e293b);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md);
  position: absolute; top: 72px; right: 24px; z-index: 10;
}
.undo-toast-text { font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); }
.undo-toast-btn {
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-primary-500); background: none; border: none; cursor: pointer; padding: 2px 4px;
  border-radius: var(--ip-radius-sm);
}
.undo-toast-btn:hover { background: var(--ip-primary-soft-bg); }

/* ===== 撤销 toast 淡入淡出 ===== */
.overlay-enter-active { animation:overlay-in 0.2s ease-out; }
.overlay-leave-active { animation:overlay-in 0.15s ease-in reverse; }
@keyframes overlay-in {
  from { opacity:0; }
  to { opacity:1; }
}
</style>
