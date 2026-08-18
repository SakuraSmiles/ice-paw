<script setup lang="ts">
// Sidebar.vue — 左侧会话列表面板
import { ref, computed, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { useProjectStore } from "../../stores/project";
import { formatDate, parseDbTime } from "../../utils/time";
import { loadLastSession, planRestore } from "../../utils/sessionRestore";
import { useNewConversation } from "../../composables/useNewConversation";
import { useTheme } from "../../composables/useTheme";
import { useResizablePanel } from "../../composables/useResizablePanel";
import PanelResizeHandle from "../common/PanelResizeHandle.vue";
import ProjectSwitcher from "./ProjectSwitcher.vue";

const router = useRouter();
const project = useProjectStore();
const isSettingsPage = computed(() => router.currentRoute.value.path.startsWith("/settings"));
// 会话选中高亮 = 「内容区正在看该会话」——非会话内容页（项目/设置）不高亮，
// 与底部设置按钮 isSettingsPage 同一语义。activeConvId 在 store 保留，
// 回首页原会话原样（高亮只是视觉降调，不丢上下文）。
const isChatRoute = computed(() => router.currentRoute.value.name === "Home");

// 会话列表 scope 唯一真相源：当前选中的项目空间（null = 散落会话）。
// 与路由解耦：在项目里点开会话去首页聊天时，侧栏仍保持该项目 scope，不会闪回散落。
const scopeProjectId = computed(() => project.activeProjectId);
const currentProjectName = computed(() => project.activeProject?.name ?? "散落会话");

// ProjectSwitcher 的 select/manage 上交处理（开关态由组件内部自持）
function selectProject(id: string | null) {
  project.setActiveProject(id);
  // 与「打开软件」一致：切到该空间最近一条会话；无会话则留在欢迎态。再回首页对话。
  const scoped = visibleConversations.value.filter((c) =>
    id === null ? !c.project_id : c.project_id === id
  );
  if (scoped.length > 0) {
    const latest = scoped.reduce((a, b) =>
      parseDbTime(b.updated_at) > parseDbTime(a.updated_at) ? b : a
    );
    chat.selectConversation(latest.id);
  } else {
    chat.clearActiveConversation();
  }
  router.push("/");
}

function gotoManage() {
  router.push("/projects");
}

/** 项目名点击 → 项目详情页（概览·任务台账/项目轨迹/设置）。不改变侧栏
 * scope——「看项目」与「切空间工作」是两个动作，回来时空间原样 */
function openProjectDetail(id: string) {
  router.push(`/projects/${id}`);
}

/** 快速新建（UX #1）：纯名字创建即切到新空间（欢迎态开聊）；完整字段留给项目页 */
async function quickCreateProject(name: string) {
  try {
    const p = await project.create({ name });
    project.setActiveProject(p.id);
    chat.clearActiveConversation();
    router.push("/");
  } catch (err) {
    console.warn("[sidebar] 快速新建项目失败", err);
  }
}

// =========================================================================
// 暗色模式（逻辑抽到 composable：本地持久化 + 系统偏好 + View Transitions + Tauri 窗口同步）
// =========================================================================
const { isDark, toggleTheme } = useTheme();

// =========================================================================
// 可调宽度（UX #2）：右缘隐形热区把手拖拽 + localStorage 记忆 + 双击重置。
// 状态/手势全在 useResizablePanel，这里只接线和边界值。
// =========================================================================
const {
  width: sidebarWidth,
  startDrag: startSidebarDrag,
  reset: resetSidebarWidth,
} = useResizablePanel({ key: "sidebar", default: 320, min: 240, max: 480, dir: 1 });

// =========================================================================
// 会话列表
// =========================================================================

import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import AgentPicker from "./AgentPicker.vue";

const chat = useChatStore();
const agent = useAgentStore();
// 新建会话逻辑（与欢迎页共用 useNewConversation，保证项目内限成员一致）
const { showPicker, pickerAgentIds, startNew, onPickAgent } = useNewConversation();
const searchQuery = ref("");

// MA-1：侧栏只显示用户会话——delegation 后台子会话不污染主列表（可见入口是
// 父会话委派卡片 / 项目页任务列表）。它们仍留在 store.conversations 里：
// 编程式 selectConversation(childId) 打开时 activeConversation 查得到。
const isUserChat = (c: { kind?: string }) => !c.kind || c.kind === "chat";
const visibleConversations = computed(() => chat.conversations.filter(isUserChat));

const scopedConversations = computed(() => {
  const pid = scopeProjectId.value;
  return pid === null
    ? visibleConversations.value.filter((c) => !c.project_id)
    : visibleConversations.value.filter((c) => c.project_id === pid);
});

const filteredConversations = computed(() => {
  if (!searchQuery.value.trim()) return scopedConversations.value;
  const q = searchQuery.value.toLowerCase();
  return scopedConversations.value.filter((c) => {
    const agentName = agent.getById(c.agent_id)?.name?.toLowerCase() ?? "";
    return c.title?.toLowerCase().includes(q) || agentName.includes(q);
  });
});

onMounted(async () => {
  agent.load();
  await project.load();
  await chat.loadConversations();
  // 启动恢复（planRestore 决策纯函数，记忆由 App.vue watch 落盘）：优先恢复
  // 「上次会话与所在页面」；持久化会话失效回退最近一条（原打开行为）；上次
  // 明确欢迎态则保持欢迎态。delegation 后台会话不作为恢复目标（用户上次
  // 主动停留的位置不该是后台子会话）。
  if (!chat.activeConvId) {
    const plan = planRestore(
      loadLastSession(),
      visibleConversations.value,
      new Set(project.activeProjects.map((p) => p.id)),
    );
    project.setActiveProject(plan.projectId);
    if (plan.convId) {
      chat.selectConversation(plan.convId);
    } else {
      chat.clearActiveConversation();
    }
    // 非首页记忆（项目详情/设置等）需要主动跳转；首页已在此无需跳
    if (plan.route && router.currentRoute.value.path === "/") {
      router.push(plan.route);
    }
  }
});

function selectConv(id: string) {
  // 非首页（设置 / 项目页）点击会话 → 回首页展示聊天
  if (router.currentRoute.value.name !== "Home") {
    router.push("/");
  }
  chat.selectConversation(id);
}

function newChat() {
  // 委托给 useNewConversation.startNew（项目内限成员、无成员引导、散落全量）
  startNew();
}

// 相对时间每分钟自动刷新：nowTick 作为 timeAgo 的响应式依赖，变化时整列重渲染
const nowTick = ref(Date.now());
let nowTickInterval: ReturnType<typeof setInterval> | null = null;
onMounted(() => {
  nowTickInterval = setInterval(() => (nowTick.value = Date.now()), 60000);
});
onUnmounted(() => {
  if (nowTickInterval) clearInterval(nowTickInterval);
});

// 取相对时间显示
function timeAgo(dateStr: string): string {
  const d = parseDbTime(dateStr);
  // 用 nowTick.value 作「现在」基准：同时建立响应式依赖，nowTick 每分钟变化时整列重算
  const now = new Date(nowTick.value);
  const diff = now.getTime() - d.getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "刚刚";
  if (mins < 60) return `${mins}分钟前`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}小时前`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}天前`;
  return formatDate(dateStr);
}
</script>

<template>
  <aside class="sidebar" :style="{ width: `${sidebarWidth}px`, minWidth: `${sidebarWidth}px` }">
    <!-- 顶部：标题 + 暗色模式切换 -->
    <div class="sidebar-header">
      <div class="sidebar-brand" role="button" tabindex="0" @click="router.push('/')" @keydown.enter="router.push('/')" @keydown.space.prevent="router.push('/')">
        <span class="brand-icon">✦</span>
        <span class="brand-name">IcePaw</span>
      </div>
      <button class="btn-theme-toggle" :title="isDark ? '切换到亮色模式' : '切换到暗色模式'" @click="toggleTheme">
        <!-- 月亮（亮色模式显示 → 点击变暗）-->
        <svg v-if="!isDark" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
        </svg>
        <!-- 太阳（暗色模式显示 → 点击变亮）-->
        <svg v-else width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="5" />
          <line x1="12" y1="1" x2="12" y2="3" />
          <line x1="12" y1="21" x2="12" y2="23" />
          <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
          <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
          <line x1="1" y1="12" x2="3" y2="12" />
          <line x1="21" y1="12" x2="23" y2="12" />
          <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
          <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
        </svg>
      </button>
    </div>

    <!-- 搜索框 -->
    <div class="sidebar-search">
      <div class="search-wrapper">
        <svg class="search-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="8" />
          <line x1="21" y1="21" x2="16.65" y2="16.65" />
        </svg>
        <input v-model="searchQuery" type="text" class="search-input" placeholder="搜索对话..." />
      </div>
    </div>

    <!-- 顶部固定区：新建对话（单行，常驻不随列表滚走）+ 项目空间胶囊（左内容右按钮） -->
    <div class="sidebar-top">
      <button class="conv-item conv-item-new" @click="newChat">
        <div class="conv-item-title">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
          <span class="conv-name">新建对话</span>
        </div>
      </button>

      <!-- 当前项目空间胶囊（开关态内部自持，select/create/manage/open 上交处理） -->
      <ProjectSwitcher
        :current-project-name="currentProjectName"
        :scope-project-id="scopeProjectId"
        :projects="project.activeProjects"
        @select="selectProject"
        @create="quickCreateProject"
        @manage="gotoManage"
        @open="openProjectDetail"
      />
    </div>

    <!-- 分隔线（钉在列表上方，不随列表滚动） -->
    <div class="conv-divider"></div>

    <!-- 会话列表（TransitionGroup：会话进出淡入、touchConversation 重排时平滑让位） -->
    <TransitionGroup name="conv-list" tag="nav" class="conv-list">
      <!-- 骨架屏只在「无可显示内容」时出现（首次加载语义）。若不加空判断，
           委派等触发的后台列表刷新会让骨架屏叠在仍可见的列表上方闪现 +
           布局下压再弹回（v-for 不在 v-if 互斥链内）——即"委派时侧栏异常动画" -->
      <div v-if="chat.convLoading && filteredConversations.length === 0" key="conv-skeleton" class="conv-skeleton">
        <div class="conv-skeleton-line" />
        <div class="conv-skeleton-line" />
        <div class="conv-skeleton-line" />
        <div class="conv-skeleton-line" />
        <div class="conv-skeleton-line" />
      </div>
      <div v-else-if="searchQuery && filteredConversations.length === 0" key="conv-empty-search" class="conv-empty">无匹配对话</div>
      <div v-else-if="!searchQuery && scopedConversations.length === 0 && agent.loaded" key="conv-empty-scope" class="conv-empty">
        {{ scopeProjectId ? "项目内暂无对话" : "暂无对话" }}
      </div>

      <button
        v-for="conv in filteredConversations"
        :key="conv.id"
        :class="['conv-item', { active: isChatRoute && chat.activeConvId === conv.id, streaming: chat.streamingConvIds.has(conv.id) }]"
        @click="selectConv(conv.id)"
      >
        <div class="conv-item-title">
          <span class="conv-name">{{ conv.title || "新对话" }}</span>
          <span v-if="conv.pinned" class="pin-icon-right" title="已置顶">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2z" /></svg>
          </span>
        </div>
        <div class="conv-meta">
          <span class="conv-agent-tag">{{ agent.getById(conv.agent_id)?.name || "未知" }}</span>
          <span v-if="chat.streamingConvIds.has(conv.id)" class="stream-indicator" title="正在生成…">
            <span class="stream-bars"><span class="bar"></span><span class="bar"></span><span class="bar"></span></span>生成中
          </span>
          <span v-else class="conv-time">{{ timeAgo(conv.updated_at) }}</span>
        </div>
      </button>
    </TransitionGroup>

    <!-- 底部：系统设置 -->
    <div class="sidebar-footer">
      <button class="footer-btn" :class="{ active: isSettingsPage }" @click="router.push('/settings/general')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        <span>设置</span>
      </button>

    </div>

    <!-- 右缘调宽把手（隐形热区，hover 显形；双击重置 320px） -->
    <PanelResizeHandle @dragstart="startSidebarDrag" @reset="resetSidebarWidth" />

    <!-- Agent 选择器弹窗 -->
    <AgentPicker v-if="showPicker" :agent-ids="pickerAgentIds" @select="onPickAgent" @close="showPicker = false" />
  </aside>
</template>

<style scoped>
.sidebar {
  /* 宽度改由 useResizablePanel 响应式绑定（内联 style）——把手 overlay 定位挂靠 */
  position: relative;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background-color: var(--color-sidebar-bg);
  border-right: 1px solid var(--color-sidebar-border);
  user-select: none;
}

/* 顶部区域 */
.sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  min-height: 68px;
  flex-shrink: 0;
}

.sidebar-brand {
  display: flex;
  align-items: center;
  gap: 8px;
}

.brand-icon {
  font-size: 20px;
  color: var(--ip-primary-500);
}

.brand-name {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

/* 暗色模式切换按钮 */
.btn-theme-toggle {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-theme-toggle:hover {
  background-color: var(--color-sidebar-item-hover);
  color: var(--ip-primary-600);
}

/* 搜索框 */
.sidebar-search {
  padding: 0 12px 12px;
  flex-shrink: 0;
}

.search-wrapper {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 34px;
  padding: 0 10px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-md);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.search-wrapper:focus-within {
  border-color: var(--color-input-focus-border);
  background-color: var(--color-input-bg);
}

.search-icon {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
}

.search-input {
  flex: 1;
  height: 100%;
  border: none;
  outline: none;
  background: transparent;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  padding: 0;
  line-height: 1;
}

.search-input::placeholder {
  color: var(--ip-color-text-tertiary);
}

/* 顶部固定区：新建对话 + 项目空间胶囊（不随会话列表滚动） */
.sidebar-top {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 0 8px 8px;
  flex-shrink: 0;
}

/* 会话列表 */
.conv-list {
  position: relative; /* leave-active 绝对定位的锚（离场项脱离流防跳动） */
  flex: 1;
  overflow-y: auto;
  padding: 0 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.conv-loading, .conv-empty {
  padding: 20px 12px;
  text-align: center;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
}

/* 骨架屏：侧栏会话列表加载中 */
.conv-skeleton { display: flex; flex-direction: column; gap: 8px; padding: 8px 12px; }
.conv-skeleton-line {
  height: 16px; border-radius: var(--ip-radius-sm);
  background: linear-gradient(90deg, var(--ip-color-bg-tertiary) 25%, var(--ip-color-bg-secondary) 50%, var(--ip-color-bg-tertiary) 75%);
  background-size: 200% 100%;
  animation: skeleton-shimmer 1.5s infinite;
}
.conv-skeleton-line:first-child { width: 70%; }
.conv-skeleton-line:last-child { width: 45%; }

@keyframes skeleton-shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

.conv-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
  padding: 10px 12px;
  text-align: left;
  border-radius: var(--ip-radius-lg);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.conv-item:hover {
  background-color: var(--color-sidebar-item-hover);
}

.conv-item.active {
  background-color: var(--color-sidebar-item-active);
}

/* 选中项左侧小圆点 */
.conv-item.active .conv-item-title::before {
  content: '';
  width: 6px;
  height: 6px;
  background-color: var(--ip-primary-500);
  border-radius: 50%;
  flex-shrink: 0;
  margin-top: 1px;
  margin-right: 2px;
}

/* 新建对话项（特殊样式，单行常驻顶部） */
.conv-item-new {
  flex-direction: row;
  align-items: center;
  padding: 8px 12px;
  border: 1px dashed var(--ip-color-border-default);
  background-color: transparent;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.conv-item-new:hover {
  border-color: var(--ip-primary-400);
  background-color: var(--ip-color-bg-tertiary);
}

.conv-item-new .conv-item-title {
  gap: 6px;
  color: var(--ip-color-primary-tint-text);
}

.conv-item-new .conv-item-title svg {
  flex-shrink: 0;
  margin-top: 0;
}

.conv-item-new .conv-name {
  color: var(--ip-color-primary-tint-text);
  font-weight: var(--ip-font-weight-medium);
}

/* 分隔线（钉在会话列表上方） */
.conv-divider {
  height: 1px;
  background-color: var(--color-sidebar-border);
  margin: 0 12px 4px;
  flex-shrink: 0;
}

.conv-item-title {
  display: flex;
  align-items: center;
  gap: 4px;
  overflow: hidden;
  width: 100%;
}

.pin-icon-right {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  margin-left: auto;
}

.conv-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.conv-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
}

.conv-agent-tag {
  font-size: 11px;
  color: var(--ip-primary-600);
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.conv-time {
  font-size: 11px;
  color: var(--ip-color-text-disabled);
  margin-left: auto;
  flex-shrink: 0;
}

/* ===== 正在生成的会话：左侧脉冲条（卡片级动画） + 「生成中」指示 ===== */
.conv-item.streaming {
  position: relative;
}
.conv-item.streaming::after {
  content: "";
  position: absolute;
  left: 0;
  top: 50%;
  width: 3px;
  height: 60%;
  border-radius: 0 3px 3px 0;
  background: var(--ip-primary-500);
  transform: translateY(-50%);
  animation: conv-stream-bar 1.4s ease-in-out infinite;
}
@keyframes conv-stream-bar {
  0%, 100% { opacity: 0.35; transform: translateY(-50%) scaleY(0.6); }
  50%      { opacity: 1;    transform: translateY(-50%) scaleY(1); }
}

.stream-indicator {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
  flex-shrink: 0;
  font-size: 11px;
  color: var(--ip-color-primary-tint-text);
}
/* 三条状「生成中」指示（依次缩放，equalizer/typing 效果） */
.stream-bars {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  height: 11px;
}
.stream-bars .bar {
  width: 2px;
  height: 100%;
  border-radius: 1px;
  background: var(--ip-primary-500);
  transform-origin: center;
  animation: stream-bar-bounce 0.9s ease-in-out infinite;
}
.stream-bars .bar:nth-child(2) { animation-delay: 0.15s; }
.stream-bars .bar:nth-child(3) { animation-delay: 0.3s; }
@keyframes stream-bar-bounce {
  0%, 100% { transform: scaleY(0.35); opacity: 0.55; }
  50%      { transform: scaleY(1);    opacity: 1; }
}

/* 暗色模式下脉冲条稍亮以保证可见 */
[data-theme='dark'] .conv-item.streaming::after {
  background: var(--ip-primary-400);
}

/* 底部 */
.sidebar-footer {
  position: relative;
  padding: 8px;
  border-top: 1px solid var(--color-sidebar-border);
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.footer-btn {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 8px 12px;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
  line-height: 1;
}

.footer-btn svg {
  display: block;
  flex-shrink: 0;
}

.footer-btn:hover,
.footer-btn.active {
  background-color: var(--color-sidebar-item-hover);
  color: var(--ip-color-text-primary);
}

</style>
