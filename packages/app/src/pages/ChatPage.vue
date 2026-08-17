<script setup lang="ts">
// ChatPage.vue — 聊天对话视图（嵌入 AppLayout 右侧主区域）
//
// 会话内标签页：标题下方常驻 [对话 | 轨迹]。发送消息是**会话级能力**，
// 输入框沉到标签页之下常驻——tab 只切换内容区的渲染形态（多 render 愿景）：
// - 对话/轨迹都是同一 session 的 render，任一 tab 都能发送；
// - 轨迹页发送 → 停留轨迹页：live 追加 + 生成中 ephemeral 行直接看进度；
// - 双 pane visibility 叠放：chat DOM 常驻 → 流式/滚动状态不因切 tab 中断
//   （display:none 会销毁布局破坏此承诺，见 .pane-hidden 注释）；
// - 切换会话 → tab 重置回「对话」；无激活会话（欢迎态）→ 标签条隐藏。
import { ref, watch } from "vue";
import ChatHeader from "../components/chat/ChatHeader.vue";
import ChatMessages from "../components/chat/ChatMessages.vue";
import ChatInput from "../components/chat/ChatInput.vue";
import ChatWelcome from "../components/chat/ChatWelcome.vue";
import AuthRequestCard from "../components/chat/AuthRequestCard.vue";
import TaskPanel from "../components/chat/TaskPanel.vue";
import TrajectoryView from "../components/trajectory/TrajectoryView.vue";
import { useChatStore } from "../stores/chat";

const chat = useChatStore();

type ChatTab = "chat" | "trajectory";
const activeTab = ref<ChatTab>("chat");

// 切会话 → 回到「对话」标签（轨迹视图自身会按新 conversationId 重载）。
// 例外：openConversationAtTrajectory 置了标志（委派卡片/项目任务列表入口）→
// 直落轨迹 tab，消费后清标志。
watch(() => chat.activeConvId, () => {
  if (chat.openTrajectoryNext) {
    chat.openTrajectoryNext = false;
    activeTab.value = "trajectory";
  } else {
    activeTab.value = "chat";
  }
});
</script>

<template>
  <div class="chat-view">
    <ChatHeader :has-tabbar="!!chat.activeConvId" />
    <ChatWelcome v-if="!chat.activeConvId" />
    <template v-else>
      <nav class="chat-tabbar">
        <button
          class="chat-tab"
          :class="{ active: activeTab === 'chat' }"
          @click="activeTab = 'chat'"
        >
          <span>对话</span>
        </button>
        <button
          class="chat-tab"
          :class="{ active: activeTab === 'trajectory' }"
          @click="activeTab = 'trajectory'"
        >
          <span>轨迹</span>
        </button>
        <!-- 会话级任务索引（本会话派生的委派任务；无任务时零占用） -->
        <TaskPanel />
      </nav>

      <!-- 内容区：tab 只切渲染形态，双 pane 常驻叠放（visibility 隐藏保布局保滚动，
           见 .pane-hidden 注释——display:none 会销毁布局：滚动归零 + 隐藏期
           scrollHeight=0 使流式跟随/挂载滚底全部失效） -->
      <div class="chat-render">
        <div class="chat-pane" :class="{ 'pane-hidden': activeTab !== 'chat' }" :aria-hidden="activeTab !== 'chat'">
          <ChatMessages />
        </div>
        <TrajectoryView
          class="traj-pane"
          :class="{ 'pane-hidden': activeTab !== 'trajectory' }"
          :aria-hidden="activeTab !== 'trajectory'"
          :conversation-id="chat.activeConvId!"
          :active="activeTab === 'trajectory'"
        />
      </div>

      <!-- 工具授权内联卡（#10 激活会话分支）：输入框上方向上弹出；
           后台会话的授权走 AppLayout 挂载的 AuthNoticeStack -->
      <AuthRequestCard />

      <!-- 输入框：会话级能力，两个 render 共用，常驻底部 -->
      <ChatInput />
    </template>
  </div>
</template>

<style scoped>
.chat-view {
  /* --msg-col-right：消息内容列右侧内边距令牌（ChatMessages 消费：气泡/日期线
     右缘 + 轮次导航条预留带）；tabbar 侧任务胶囊也用它对齐气泡右缘（见
     .chat-tabbar 内规则）。调轨道宽度只改这一个值。 */
  --msg-col-right: 80px;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
}

/* 标签条与 ChatHeader 视觉一体：同底色（明暗各自跟随 --color-chat-header-bg），
   无顶部分割线（header 侧配套去掉底边线，见 ChatHeader 的 has-tabbar 修饰）。
   ⚠️ position+z-index 必须保留：tabbar 是非定位元素时，其内任务胶囊 popover
   的 z-index 只在 tabbar 子树内生效；而下方 .chat-render 的双 pane 是
   position:absolute 定位元素，绘制在所有非定位元素之上 → popover 被内容区
   整层盖住。z-index:20 需高于内容区内部最高层（轨迹页 10 / 撤销 toast 10）。 */
.chat-tabbar {
  position: relative;
  z-index: 20;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 24px;
  background: var(--color-chat-header-bg);
  backdrop-filter: blur(8px);
  flex-shrink: 0;
}

/* 任务胶囊右移对齐（用户 2026-08-17）：胶囊右缘与消息区用户气泡右缘对齐
   （= 滚动条槽 6px + --msg-col-right），展开的 popover（right:0 锚胶囊右缘）
   不再遮右侧轮次导航条（其占右缘 35~57px 带）。24px = tabbar 右内边距。 */
.chat-tabbar :deep(.task-panel) { margin-right: calc(var(--msg-col-right) + 6px - 24px); }

.chat-tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 20px;
  border: none;
  background: none;
  cursor: pointer;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  border-bottom: 2px solid transparent;
  transition: color var(--ip-duration-fast) var(--ip-ease-out), border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.chat-tab:hover { color: var(--ip-color-text-secondary); }
.chat-tab.active { color: var(--ip-primary-600); border-bottom-color: var(--ip-primary-500); font-weight: var(--ip-font-weight-medium); }

/* 内容区（render 切换区）：tab 只决定这里渲染什么，输入框在其下常驻 */
.chat-render {
  flex: 1;
  min-height: 0;
  display: flex;
  position: relative;
}

/* 双 pane 绝对定位叠放：切 tab 只切 visibility，两 pane 布局常驻。 */
.chat-pane {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.traj-pane {
  position: absolute;
  inset: 0;
  min-width: 0;
}

/* 隐藏 pane 用 visibility 而非 v-show（display:none）：
   - display:none 销毁布局 → 滚动位置归零（切回 tab 从顶部开始），
     且隐藏期 scrollHeight=0 → 流式自动跟随 / 挂载期滚底全是 no-op；
   - visibility:hidden 保布局保滚动 → 「chat DOM 常驻，流式/滚动状态不因
     切 tab 中断」的设计承诺真正成立；不可聚焦不可交互 */
.pane-hidden {
  visibility: hidden;
  pointer-events: none;
}
</style>
