<script setup lang="ts">
// Sidebar.vue — 左侧会话列表面板
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { useProjectStore } from "../../stores/project";
import { parseDbTime, timeAgo } from "../../utils/time";
import { loadLastSession, planRestore } from "../../utils/sessionRestore";
import { useNewConversation } from "../../composables/useNewConversation";
import { useTheme } from "../../composables/useTheme";
import { useResizablePanel } from "../../composables/useResizablePanel";
import { useInbox } from "../../composables/useInbox";
import { listen } from "@tauri-apps/api/event";
import { bridge } from "../../api/bridge";
import PanelResizeHandle from "../common/PanelResizeHandle.vue";
import EntityAvatar from "../common/EntityAvatar.vue";
import ProjectSwitcher from "./ProjectSwitcher.vue";
import { PanelLeftClose, PanelLeftOpen, MessageSquarePlus, MessagesSquare, Settings, Hash, Star } from "@lucide/vue";
import { useEscapeStack } from "../../composables/useEscapeStack";

const router = useRouter();
const project = useProjectStore();
// MA-3 收件箱计数（会话行 badge；hold/accept 来件都计——hold 待批准是主场景，
// accept 排队中也值得看见「有几件在等」）
const { pendingOf } = useInbox();
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
  if (id !== null) {
    // 切项目 → 直达项目详情页（2026-08-18 用户拍板）：先看台账/轨迹再进会话；
    // 会话上下文不动，从详情页点侧栏会话照常回首页
    router.push(`/projects/${id}`);
    return;
  }
  // 散落：选最近一条会话回首页（无会话留欢迎态）
  const scoped = visibleConversations.value.filter((c) => !c.project_id);
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
// 收起/展开（rail 模式，2026-09-01）：56px 单列图标行动栏。
// - 持久化：布尔旗标（先例 icepaw-traj-duration）；sidebarWidth 值不动，展开还原。
// - 动画：width 过渡只在 .animating 的 300ms 窗口内存在——无条件挂会把
//   useResizablePanel 拖拽的逐帧改宽拖成橡皮筋。.animating 同时挂
//   overflow:hidden（展开树挂载即全宽，随容器长出）；休息态必须移除，
//   flyout 是 absolute 伸出侧栏外，休息态不得裁剪。
// =========================================================================
const COLLAPSED_KEY = "icepaw-sidebar-collapsed";
const RAIL_WIDTH = 56;

const collapsed = ref(localStorage.getItem(COLLAPSED_KEY) === "1");
watch(collapsed, (v) => localStorage.setItem(COLLAPSED_KEY, v ? "1" : "0"));

const animating = ref(false);
let animTimer: ReturnType<typeof setTimeout> | null = null;

/** 动画窗口时长：reduced-motion 下全局把 transition 归零（global.css），
 *  定时器同步缩到 0——否则 overflow:hidden 空挂 300ms，期间 flyout 伸不出去 */
function animDurationMs(): number {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 300;
}

function toggleCollapsed() {
  if (convFlyoutOpen.value) closeConvFlyout();
  if (animTimer) clearTimeout(animTimer);
  animating.value = true; // 先挂类（overflow + transition），同 tick 换宽换树，
  collapsed.value = !collapsed.value; // Vue 批更新只布局一次，transition 从旧宽起跑
  animTimer = setTimeout(() => {
    animating.value = false;
    animTimer = null;
  }, animDurationMs());
}

/** aside 宽度绑定：收起 = 固定 56px；展开 = 可调宽（收起期间 sidebarWidth 原值保留） */
const sidebarStyle = computed(() => {
  const w = collapsed.value ? RAIL_WIDTH : sidebarWidth.value;
  return { width: `${w}px`, minWidth: `${w}px` };
});

// =========================================================================
// 会话 flyout（收起态的会话列表入口，不展开侧栏）：常驻 DOM + .open class 开合
// （沿用 ProjectSwitcher 模式——Transition+v-if 首开不挂载的真机事故在案）。
// 搜索复用同一个 searchQuery（展开态搜索框与 flyout 搜索框互斥可见，无竞争）。
// =========================================================================
const convFlyoutOpen = ref(false);
const flyoutMenuRef = ref<HTMLDivElement | null>(null);
const convBtnRef = ref<HTMLButtonElement | null>(null);

function openConvFlyout() {
  convFlyoutOpen.value = true;
  nextTick(() => flyoutMenuRef.value?.focus()); // a11y 基线：焦点入浮层
}
/** restoreFocus 仅在「用户交互关」（Esc / 点遮罩）时归还焦点给触发钮；
 *  编程关闭（选中会话/路由切换/收起侧栏）不抢焦点——rail 树可能正在卸载 */
function closeConvFlyout(restoreFocus = false) {
  convFlyoutOpen.value = false;
  if (restoreFocus) convBtnRef.value?.focus();
}
function toggleConvFlyout() {
  if (convFlyoutOpen.value) closeConvFlyout(true);
  else openConvFlyout();
}
function selectConvFromFlyout(id: string) {
  closeConvFlyout();
  selectConv(id); // 复用：非首页先 router.push("/") 再选中
}

// Esc：全局关闭栈（Sidebar 常驻挂载 → 条目恒在栈底，active 谓词让路——flyout
// 关着时不消费 Esc，放行给组件自身的 Esc 处理，如标题编辑输入框）
useEscapeStack(() => closeConvFlyout(true), () => convFlyoutOpen.value);

// 切路由关 flyout（不还焦点）：菜单语义绑定当前内容语境，路由已变语境即失效
watch(
  () => router.currentRoute.value.fullPath,
  () => {
    if (convFlyoutOpen.value) closeConvFlyout();
  },
);

// =========================================================================
// 会话列表
// =========================================================================

import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import AgentPicker from "./AgentPicker.vue";

const chat = useChatStore();
const agent = useAgentStore();
// 新建会话逻辑（与欢迎页共用 useNewConversation，保证项目内限成员一致）
const { showPicker, pickerAgentIds, ctaKind, startNew, onPickAgent } = useNewConversation();

// MA-1：侧栏只显示用户会话——delegation 后台子会话不污染主列表（可见入口是
// 父会话委派卡片 / 项目页任务列表）。它们仍留在 store.conversations 里：
// 编程式 selectConversation(childId) 打开时 activeConversation 查得到。
const isUserChat = (c: { kind?: string }) => !c.kind || c.kind === "chat";
const visibleConversations = computed(() => chat.conversations.filter(isUserChat));

// 启动恢复候选 = 侧栏可见会话 + 频道会话——频道在侧栏由独立区块渲染（不进
// visibleConversations），但它是用户主动停留的页面：关闭时停在频道、重启
// 却因候选集缺频道而回退「最新会话」（生产实案 2026-09-11）。delegation
// 后台子会话仍排除（用户上次主动停留的位置不该是后台子会话）。
const restorableConversations = computed(() =>
  chat.conversations.filter((c) => isUserChat(c) || c.kind === "channel"),
);

const scopedConversations = computed(() => {
  const pid = scopeProjectId.value;
  return pid === null
    ? visibleConversations.value.filter((c) => !c.project_id)
    : visibleConversations.value.filter((c) => c.project_id === pid);
});

// =========================================================================
// 频道 v1（§6.1 侧栏形态）：项目 scope = 活频道一行（未开启给「开启频道」
// 懒建入口）；散落 scope = 归档频道列表（原项目已删除的只读频道，无则区块
// 隐藏）。频道不进 isUserChat 过滤（kind='channel'），恒由本区块独占渲染。
// =========================================================================
/** 当前项目 scope 的活频道（部分唯一索引保证至多一条；null = 未开启） */
const scopeChannel = computed(() => {
  const pid = scopeProjectId.value;
  if (pid === null) return null;
  return (
    chat.conversations.find(
      (c) => c.kind === "channel" && c.project_id === pid && !c.archived_at,
    ) ?? null
  );
});

/** 散落 scope 的归档频道（项目删除 → FK SET NULL 转散落 + archived_at 落时间） */
const archivedChannels = computed(() =>
  chat.conversations.filter((c) => c.kind === "channel" && !c.project_id),
);

const channelEnsuring = ref(false);
const channelEnsureError = ref<string | null>(null);

/** 开启（或选中）当前项目的频道：懒建 ensure → 刷新列表 → 选中。失败文案
 *  走行内 caption（后端三段式，如「项目还没有成员」）——侧栏无全局 toast
 *  先例，不为此新建机制。 */
async function openChannel() {
  const pid = scopeProjectId.value;
  if (pid === null || channelEnsuring.value) return;
  const existing = scopeChannel.value;
  if (existing) {
    selectConv(existing.id);
    return;
  }
  channelEnsuring.value = true;
  channelEnsureError.value = null;
  try {
    const view = await bridge.channels.ensure(pid);
    if (view.channel) {
      await chat.loadConversations();
      selectConv(view.channel.id);
    }
  } catch (e) {
    console.error("开启频道失败:", e);
    channelEnsureError.value = e instanceof Error ? e.message : String(e);
  } finally {
    channelEnsuring.value = false;
  }
}

/** 频道行 meta：统筹者。权威源 = project.agents 的 role='coordinator'（与
 *  后端 build_view / 路由同源）——**勿读 conv.agent_id**：ensure_channel 的
 *  投影在未选举时刻意回落 joined_at 最早成员（临时投影不阻塞使用），拿它当
 *  统筹者 = 选举前谎报名字（生产实案 2026-09-11 侧栏误显）。 */
const scopeCoordinator = computed(() => {
  const pid = scopeProjectId.value;
  if (pid === null) return null;
  const id = project.getById(pid)?.agents?.find((a) => a.role === "coordinator")?.agent_id;
  return id ? (agent.getById(id) ?? null) : null;
});

const channelCoordinatorName = computed(() => scopeCoordinator.value?.name ?? "待选举");

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
      restorableConversations.value,
      new Set(project.activeProjects.map((p) => p.id)),
      new Set(project.list.map((p) => p.id)), // 全量（含归档）——route 守卫判「已永久删除」
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
// 统筹位落定回正（channel_coordinator 事件：elected/appointed/removed/failed-over
// 四动作都发）→ 强制刷新项目列表。agents[].role 是侧栏统筹者显示的权威源，
// 选举完成时侧栏常驻可见，不回正会一直挂着过期的「待选举」/旧名字。
let unlistenCoordEvent: (() => void) | null = null;
onMounted(async () => {
  nowTickInterval = setInterval(() => (nowTick.value = Date.now()), 60000);
  unlistenCoordEvent = await listen<{ conversation_id: string; kind: string }>(
    "session:event-appended",
    (e) => {
      if (e.payload?.kind === "channel_coordinator") void project.load(true);
    },
  );
});
onUnmounted(() => {
  if (nowTickInterval) clearInterval(nowTickInterval);
  unlistenCoordEvent?.();
  if (animTimer) clearTimeout(animTimer);
});

// 取相对时间显示（timeAgo 抽至 utils/time 共用；nowTick 作「现在」基准并
// 建立响应式依赖，每分钟变化时整列重算）
function timeAgoLabel(dateStr: string): string {
  return timeAgo(dateStr, nowTick.value);
}
</script>

<template>
  <aside class="sidebar" :class="{ collapsed, animating }" :style="sidebarStyle">
    <!-- 顶部：标题 + 收起侧栏（主题钮已移 footer，2026-09-01） -->
    <div v-if="!collapsed" class="sidebar-header">
      <div class="sidebar-brand" role="button" tabindex="0" @click="router.push('/')" @keydown.enter="router.push('/')" @keydown.space.prevent="router.push('/')">
        <span class="brand-icon">✦</span>
        <span class="brand-name">IcePaw</span>
      </div>
      <button class="btn-icon" title="收起侧边栏" @click="toggleCollapsed">
        <PanelLeftClose :size="20" />
      </button>
    </div>

    <!-- 顶部固定区（顺序拍板 2026-09-10）：项目空间胶囊 → 项目频道 → 新建对话。
         搜索框已随频道改版整体移除（无使用场景；rail flyout 内同步移除）。 -->
    <div v-if="!collapsed" class="sidebar-top">
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

      <!-- 项目频道（§6.1）：活频道一行 / 未开启给「开启频道」懒建入口；
           归档频道只在散落 scope 出现（原项目已删除的只读记录） -->
      <div v-if="scopeProjectId" class="channel-block">
        <button
          v-if="scopeChannel"
          :class="['conv-item', 'channel-item', { active: isChatRoute && chat.activeConvId === scopeChannel.id, streaming: chat.streamingConvIds.has(scopeChannel.id) }]"
          @click="selectConv(scopeChannel.id)"
        >
          <div class="conv-item-title">
            <Hash :size="14" class="channel-hash" aria-hidden="true" />
            <span class="conv-name">{{ scopeChannel.title || "项目频道" }}</span>
          </div>
          <div class="conv-meta">
            <span class="conv-agent-tag">
              <EntityAvatar
                v-if="scopeCoordinator"
                class="conv-agent-avatar"
                :name="scopeCoordinator.name"
                :image="scopeCoordinator.avatar ?? null"
                size="xs"
              />
              <span class="conv-agent-name">统筹 · {{ channelCoordinatorName }}</span>
            </span>
            <span v-if="chat.streamingConvIds.has(scopeChannel.id)" class="stream-indicator" title="正在生成…">
              <span class="stream-bars"><span class="bar"></span><span class="bar"></span><span class="bar"></span></span>生成中
            </span>
            <span v-else class="conv-time">{{ timeAgoLabel(scopeChannel.updated_at) }}</span>
          </div>
        </button>
        <button
          v-else
          class="conv-item conv-item-new channel-open"
          :disabled="channelEnsuring"
          @click="openChannel"
        >
          <div class="conv-item-title">
            <Hash :size="16" aria-hidden="true" />
            <span class="conv-name">{{ channelEnsuring ? "开启中…" : "开启频道" }}</span>
          </div>
        </button>
        <p v-if="channelEnsureError" class="channel-error">{{ channelEnsureError }}</p>
      </div>
      <template v-else-if="archivedChannels.length > 0">
        <button
          v-for="c in archivedChannels"
          :key="c.id"
          :class="['conv-item', 'channel-item', { active: isChatRoute && chat.activeConvId === c.id }]"
          @click="selectConv(c.id)"
        >
          <div class="conv-item-title">
            <Hash :size="14" class="channel-hash" aria-hidden="true" />
            <span class="conv-name">{{ c.title || "项目频道" }}</span>
            <span class="channel-archived-tag">已归档</span>
          </div>
          <div class="conv-meta">
            <span class="conv-time">{{ timeAgoLabel(c.updated_at) }}</span>
          </div>
        </button>
      </template>

      <button class="conv-item conv-item-new" @click="newChat">
        <div class="conv-item-title">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
          <span class="conv-name">新建对话</span>
        </div>
      </button>
    </div>

    <!-- 分隔线（钉在列表上方，不随列表滚动） -->
    <div v-if="!collapsed" class="conv-divider"></div>

    <!-- 会话列表（TransitionGroup：会话进出淡入、touchConversation 重排时平滑让位） -->
    <TransitionGroup v-if="!collapsed" name="conv-list" tag="nav" class="conv-list">
      <!-- 骨架屏只在「无可显示内容」时出现（首次加载语义）。若不加空判断，
           委派等触发的后台列表刷新会让骨架屏叠在仍可见的列表上方闪现 +
           布局下压再弹回（v-for 不在 v-if 互斥链内）——即"委派时侧栏异常动画" -->
      <div v-if="chat.convLoading && scopedConversations.length === 0" key="conv-skeleton" class="conv-skeleton">
        <div class="conv-skeleton-line" />
        <div class="conv-skeleton-line" />
        <div class="conv-skeleton-line" />
        <div class="conv-skeleton-line" />
        <div class="conv-skeleton-line" />
      </div>
      <div v-else-if="scopedConversations.length === 0 && agent.loaded" key="conv-empty-scope" class="conv-empty">
        <!-- 空态即引导：全新用户给出下一步方向，「新建对话」按钮此刻的行为就是去创建智能体 -->
        <template v-if="ctaKind === 'no-agents'">还没有智能体——点上方「新建对话」先创建一个</template>
        <template v-else>{{ scopeProjectId ? "项目内暂无对话" : "暂无对话" }}</template>
      </div>

      <button
        v-for="conv in scopedConversations"
        :key="conv.id"
        :class="['conv-item', { active: isChatRoute && chat.activeConvId === conv.id, streaming: chat.streamingConvIds.has(conv.id) }]"
        @click="selectConv(conv.id)"
      >
        <div class="conv-item-title">
          <span class="conv-name">{{ conv.title || "新对话" }}</span>
          <!-- MA-3 来件 badge（主色圆点+计数；与 rail flyout 内同款两处同步改） -->
          <span v-if="pendingOf(conv.id) > 0" class="conv-inbox-badge" :title="`收件箱：${pendingOf(conv.id)} 条待处理来件`">
            {{ pendingOf(conv.id) > 9 ? "9+" : pendingOf(conv.id) }}
          </span>
          <span v-if="conv.pinned" class="pin-icon-right" title="已置顶">
            <!-- 置顶星（填充形态，与 rail flyout 内同款两处同步改） -->
            <Star :size="11" fill="currentColor" stroke="none" aria-hidden="true" />
          </span>
        </div>
        <div class="conv-meta">
          <span class="conv-agent-tag">
            <EntityAvatar
              class="conv-agent-avatar"
              :name="agent.getById(conv.agent_id)?.name || '?'"
              :image="agent.getById(conv.agent_id)?.avatar ?? null"
              size="xs"
            />
            <span class="conv-agent-name">{{ agent.getById(conv.agent_id)?.name || "未知" }}</span>
          </span>
          <span v-if="chat.streamingConvIds.has(conv.id)" class="stream-indicator" title="正在生成…">
            <span class="stream-bars"><span class="bar"></span><span class="bar"></span><span class="bar"></span></span>生成中
          </span>
          <span v-else class="conv-time">{{ timeAgoLabel(conv.updated_at) }}</span>
        </div>
      </button>
    </TransitionGroup>

    <!-- 底部：设置（左）+ 明暗切换（右，2026-09-01 自 header 迁入——圆形扩散
         动画起点自动跟随按钮新位置，useTheme 按 .btn-theme-toggle 探测零改动。
         收起态不放主题钮（用户拍板）：此份即全应用唯一实例） -->
    <div v-if="!collapsed" class="sidebar-footer">
      <button class="footer-btn" :class="{ active: isSettingsPage }" @click="router.push('/settings/general')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        <span>设置</span>
      </button>

      <!-- 主题钮 markup 整体迁入（class 名/图标不动，保扩散起点探测；v-if/v-else
           互斥树保证全应用单实例——收起树无主题钮，此份恒唯一） -->
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

    <!-- ===== 收起态：56px 单列行动栏（与上方展开树 v-if 互斥；两态 markup 同款
         的部分各带「同款同步改」注释互锚，刻意不抽组件保 scoped CSS 直接复用）===== -->
    <div v-else class="sidebar-rail">
      <!-- header：单占位展开钮（收起态不保品牌位，用户拍板 2026-09-01；
           居中与行动区按钮同列） -->
      <div class="sidebar-header rail-header">
        <button class="btn-icon" title="展开侧边栏" @click="toggleCollapsed">
          <PanelLeftOpen :size="20" />
        </button>
      </div>

      <!-- 行动区（顺序对齐展开态：项目空间 → 项目频道 → 新建 / 会话列表入口） -->
      <div class="rail-actions">
        <!-- 项目空间：收起变体（32px 图标钮 + 菜单向右弹；逻辑/emit 原班复用） -->
        <ProjectSwitcher
          collapsed
          :current-project-name="currentProjectName"
          :scope-project-id="scopeProjectId"
          :projects="project.activeProjects"
          @select="selectProject"
          @create="quickCreateProject"
          @manage="gotoManage"
          @open="openProjectDetail"
        />

        <!-- 项目频道（散落 scope 隐藏——归档频道无生成态，rail 不给入口；
             展开侧栏可从「已归档」列表进入） -->
        <button
          v-if="scopeProjectId"
          class="btn-icon"
          :class="{ active: isChatRoute && scopeChannel && chat.activeConvId === scopeChannel.id }"
          title="项目频道"
          @click="openChannel"
        >
          <Hash :size="20" />
        </button>

        <button class="btn-icon" title="新建对话" @click="newChat">
          <MessageSquarePlus :size="20" />
        </button>

        <div class="rail-flyout">
          <button
            ref="convBtnRef"
            class="btn-icon"
            :class="{ active: convFlyoutOpen }"
            title="会话列表"
            aria-haspopup="dialog"
            :aria-expanded="convFlyoutOpen"
            @click="toggleConvFlyout"
          >
            <MessagesSquare :size="20" />
          </button>

          <!-- 点外关闭：fixed 遮罩（先例 ProjectSwitcher.switcher-overlay） -->
          <div class="flyout-overlay" :class="{ open: convFlyoutOpen }" @click="closeConvFlyout(true)" />

          <div
            ref="flyoutMenuRef"
            class="flyout-menu"
            :class="{ open: convFlyoutOpen }"
            role="dialog"
            aria-label="会话列表"
            tabindex="-1"
            :aria-hidden="!convFlyoutOpen || undefined"
          >
            <div class="flyout-list">
              <div v-if="chat.convLoading && scopedConversations.length === 0" class="conv-skeleton">
                <div class="conv-skeleton-line" />
                <div class="conv-skeleton-line" />
                <div class="conv-skeleton-line" />
                <div class="conv-skeleton-line" />
              </div>
              <div v-else-if="scopedConversations.length === 0 && agent.loaded" class="conv-empty">
                {{ scopeProjectId ? "项目内暂无对话" : "暂无对话" }}
              </div>
              <!-- ⚠️ 会话项与展开列表（上方 TransitionGroup 内）完全同款，两处同步改 -->
              <button
                v-for="conv in scopedConversations"
                :key="conv.id"
                :class="['conv-item', { active: isChatRoute && chat.activeConvId === conv.id, streaming: chat.streamingConvIds.has(conv.id) }]"
                @click="selectConvFromFlyout(conv.id)"
              >
                <div class="conv-item-title">
                  <span class="conv-name">{{ conv.title || "新对话" }}</span>
                  <!-- MA-3 来件 badge（与展开列表同款两处同步改） -->
                  <span v-if="pendingOf(conv.id) > 0" class="conv-inbox-badge" :title="`收件箱：${pendingOf(conv.id)} 条待处理来件`">
                    {{ pendingOf(conv.id) > 9 ? "9+" : pendingOf(conv.id) }}
                  </span>
                  <span v-if="conv.pinned" class="pin-icon-right" title="已置顶">
                    <!-- 置顶星（填充形态，与展开列表同款两处同步改） -->
                    <Star :size="11" fill="currentColor" stroke="none" aria-hidden="true" />
                  </span>
                </div>
                <div class="conv-meta">
                  <span class="conv-agent-tag">
                    <EntityAvatar
                      class="conv-agent-avatar"
                      :name="agent.getById(conv.agent_id)?.name || '?'"
                      :image="agent.getById(conv.agent_id)?.avatar ?? null"
                      size="xs"
                    />
                    <span class="conv-agent-name">{{ agent.getById(conv.agent_id)?.name || "未知" }}</span>
                  </span>
                  <span v-if="chat.streamingConvIds.has(conv.id)" class="stream-indicator" title="正在生成…">
                    <span class="stream-bars"><span class="bar"></span><span class="bar"></span><span class="bar"></span></span>生成中
                  </span>
                  <span v-else class="conv-time">{{ timeAgoLabel(conv.updated_at) }}</span>
                </div>
              </button>
            </div>
          </div>
        </div>
      </div>

      <!-- footer：设置单钮（主题钮收起态不出现，用户拍板 2026-09-01——展开态
           footer 那份即全应用唯一实例；收起态切主题 = 展开侧栏再切） -->
      <div class="sidebar-footer rail-footer">
        <button class="btn-icon" :class="{ active: isSettingsPage }" title="设置" @click="router.push('/settings/general')">
          <Settings :size="20" />
        </button>
      </div>
    </div>

    <!-- 右缘调宽把手（隐形热区，hover 显形；双击重置 320px；收起态无宽度可调故隐藏，
         sidebarWidth 值不动——展开即还原用户上次拖的宽度） -->
    <PanelResizeHandle v-if="!collapsed" @dragstart="startSidebarDrag" @reset="resetSidebarWidth" />

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
  background-color: var(--ip-color-bg-primary);
  border-right: 1px solid var(--ip-color-border-default);
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
  gap: var(--ip-spacing-2);
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
  background-color: var(--ip-color-bg-sidebar-item-hover);
  color: var(--ip-primary-600);
}

/* 顶部固定区：项目空间胶囊 + 项目频道 + 新建对话（不随会话列表滚动） */
.sidebar-top {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 0 8px 8px;
  flex-shrink: 0;
}

/* ===== 频道 v1 ===== */
.channel-block {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

/* Hash 图标进文本流：base.css 全局 svg display:block reset 陷阱——必须显式
   inline-block，否则独占一行（84aa6c5 轨迹表同款） */
.channel-hash {
  display: inline-block;
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
}

/* 归档频道「已归档」标（micro 胶囊，语义中性灰） */
.channel-archived-tag {
  flex-shrink: 0;
  margin-left: auto;
  padding: 1px 6px;
  border-radius: var(--ip-radius-full, 999px);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-micro-size);
  line-height: 1.4;
}

/* 「开启频道」失败的行内错误文案（后端三段式原文；常驻至下次尝试成功） */
.channel-error {
  margin: 2px 4px 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-danger-text);
  line-height: 1.5;
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
.conv-skeleton { display: flex; flex-direction: column; gap: var(--ip-spacing-2); padding: 8px 12px; }
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
  background-color: var(--ip-color-bg-sidebar-item-hover);
}

.conv-item.active {
  background-color: var(--ip-color-bg-sidebar-item-active);
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
  background-color: var(--ip-color-border-default);
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
/* MA-3 来件 badge：主色圆点胶囊（margin-left auto 推到行尾；与置顶星并存时
   badge 在前星在后——待办数字比置顶态更 actionable） */
.conv-inbox-badge {
  flex-shrink: 0;
  margin-left: auto;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--ip-radius-full, 999px);
  background: var(--ip-primary-500);
  color: #fff;
  font-size: var(--ip-text-micro-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: 1;
}
/* badge 在场时置顶星不再单独推尾（badge 已 margin-left:auto） */
.conv-item-title:has(.conv-inbox-badge) .pin-icon-right { margin-left: 2px; }

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
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-primary-600);
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  min-width: 0;
  overflow: hidden;
}
/* agent 小头像（xs=16px，EntityAvatar 三级链） */
.conv-agent-avatar { flex-shrink: 0; }
/* 名字层（ellipsis 须落在文本节点所在元素上，flex 容器自身省略无效） */
.conv-agent-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.conv-time {
  font-size: var(--ip-text-micro-size);
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
  font-size: var(--ip-text-micro-size);
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

/* 底部：展开态单行（设置左 + 主题钮右，2026-09-01 主题钮自 header 迁入；
   rail 态由 .rail-footer 覆盖回纵向） */
.sidebar-footer {
  position: relative;
  padding: 8px;
  border-top: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-1);
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
  background-color: var(--ip-color-bg-sidebar-item-hover);
  color: var(--ip-color-text-primary);
}

/* ============================================================
   收起态（rail）：56px 单列行动栏
   ============================================================ */

/* 通用图标钮（收起/展开/新建/会话入口/rail 设置，36px 方形、token 化悬停——
   2026-09-01 二轮：32→36 + 图标 18→20，每边留白 12→10px；56 轨宽不动） */
.btn-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  flex-shrink: 0;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    color var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-icon:hover {
  background-color: var(--ip-color-bg-sidebar-item-hover);
  color: var(--ip-primary-600);
}

.btn-icon.active {
  background-color: var(--ip-color-bg-sidebar-item-active);
  color: var(--ip-color-text-primary);
}

/* 宽度过渡只存在于动画窗口（toggleCollapsed 挂 .animating 300ms）：
   无条件挂 transition 会把调宽把手的逐帧改宽拖成橡皮筋；overflow:hidden 同理
   只在此窗口——休息态会话 flyout（absolute）要能伸出侧栏右缘外 */
.sidebar.animating {
  overflow: hidden;
  transition:
    width var(--ip-duration-panel) var(--ip-ease-out),
    min-width var(--ip-duration-panel) var(--ip-ease-out);
}

.sidebar-rail {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

/* rail header：单占位展开钮居中（与行动区按钮同列；收起态不保品牌位） */
.rail-header {
  padding: 14px var(--ip-spacing-0_5);
  justify-content: center;
}

/* 行动区：纵向居中一列（新建 / 会话 flyout / 项目 flyout），中段弹性占位 */
.rail-actions {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2) var(--ip-spacing-1);
}

/* rail footer：覆盖展开态的 row/space-between 回纵向中轴（设置上、主题下） */
.sidebar-footer.rail-footer {
  flex-direction: column;
  justify-content: flex-start;
  align-items: center;
}

/* 会话 flyout：常驻 DOM + .open class 开合（ProjectSwitcher 事故先例——
   Transition+v-if 首开不挂载；关态三件套防幽灵命中） */
.rail-flyout {
  position: relative;
  display: flex;
}

.flyout-overlay {
  position: fixed;
  inset: 0;
  z-index: var(--ip-z-dropdown);
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  transition:
    opacity var(--ip-duration-fast) var(--ip-ease-out),
    visibility var(--ip-duration-fast);
}

.flyout-overlay.open {
  opacity: 1;
  visibility: visible;
  pointer-events: auto;
}

.flyout-menu {
  position: absolute;
  left: calc(100% + var(--ip-spacing-2));
  top: 0;
  z-index: calc(var(--ip-z-dropdown) + 1);
  width: 300px;
  max-height: min(420px, 60vh);
  display: flex;
  flex-direction: column;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  transform: translateX(-6px) scaleY(0.98);
  transform-origin: top left;
  transition:
    opacity var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out),
    visibility var(--ip-duration-fast);
}

.flyout-menu.open {
  opacity: 1;
  visibility: visible;
  pointer-events: auto;
  transform: none;
}

/* flyout 列表：限高内滚（骨架/空态/会话项 class 全复用展开态那套） */
.flyout-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--ip-spacing-2);
  display: flex;
  flex-direction: column;
  gap: 2px;
}

</style>
