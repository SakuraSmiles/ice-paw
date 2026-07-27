<script setup lang="ts">
// SkeletonScreen.vue — REQ-XC-007 应用加载骨架屏
//
// 职责：
//   - 在 App.vue 用 <Suspense> 包裹整个 router-view 时，作为 `v-slot:fallback` 渲染
//   - 视觉上 1:1 复刻 AppShell（Sidebar + 主内容区）的轮廓，让用户感觉「应用正在加载」
//   - 通过 shimmer 动画（左右扫光）传达「在动」的状态
//   - 不依赖任何 store / 网络，纯静态展示组件
//
// 设计要点：
//   - 侧边栏骨架：项目选择器占位 + 5 条会话列表占位 + 底部 NewChatButton 占位
//   - 聊天区骨架：顶部 header 占位 + 3 条消息气泡占位（user + AI 交替）+ 底部输入区占位
//   - 全部占位元素使用 `--ip-color-bg-tertiary` 背景 + `::after` shimmer 扫光
//   - 与 AppLayout 严格保持 260px 侧边栏 + flex 主内容区，避免布局跳动
//
// props: 无（纯展示）

// ============================================================================
// 常量
// ============================================================================

/** 会话列表骨架条数（视觉上接近默认的 5-7 条） */
const CONVERSATION_SKELETON_COUNT = 6;

/** 消息气泡骨架条数（交替 user / AI 各 3 条） */
const MESSAGE_SKELETON_COUNT = 6;

// ============================================================================
// 模板
// ============================================================================
</script>

<template>
  <!-- 骨架屏根容器：与 AppLayout.app-shell 完全同构（display:flex + 高度 100vh） -->
  <div class="skeleton-shell" role="status" aria-label="应用加载中" aria-live="polite">
    <!-- 左侧：Sidebar 骨架（260px 固定宽度） -->
    <aside class="skeleton-sidebar" aria-hidden="true">
      <!-- 项目选择器占位 -->
      <div class="skel-block skel-project-selector">
        <div class="skel-line skel-line-lg" />
        <div class="skel-line skel-line-sm" />
      </div>

      <!-- NewChatButton 占位 -->
      <div class="skel-block skel-new-chat">
        <div class="skel-line skel-line-md" />
      </div>

      <!-- 会话列表占位 -->
      <div class="skel-conversation-list">
        <div
          v-for="i in CONVERSATION_SKELETON_COUNT"
          :key="`conv-${i}`"
          class="skel-conversation-item"
        >
          <div class="skel-line skel-line-md" />
          <div class="skel-line skel-line-xs" />
        </div>
      </div>

      <!-- 底部留白占位（撑高以匹配真实 Sidebar 底部按钮区） -->
      <div class="skel-sidebar-footer">
        <div class="skel-line skel-line-sm" />
      </div>
    </aside>

    <!-- 右侧：主内容区骨架（flex: 1） -->
    <main class="skeleton-main" aria-hidden="true">
      <!-- 顶部 header 占位（48px 高，与 ChatHeader 同高） -->
      <div class="skel-header">
        <div class="skel-line skel-line-avatar" />
        <div class="skel-header-text">
          <div class="skel-line skel-line-md" />
          <div class="skel-line skel-line-xs" />
        </div>
        <div class="skel-line skel-line-btn" />
      </div>

      <!-- 消息列表骨架（flex: 1） -->
      <div class="skel-message-list">
        <div
          v-for="i in MESSAGE_SKELETON_COUNT"
          :key="`msg-${i}`"
          :class="['skel-message', i % 2 === 1 ? 'skel-message-user' : 'skel-message-ai']"
        >
          <div class="skel-bubble">
            <div class="skel-line skel-line-bubble-lg" />
            <div class="skel-line skel-line-bubble-md" />
            <div v-if="i % 3 === 0" class="skel-line skel-line-bubble-sm" />
          </div>
        </div>
      </div>

      <!-- 底部输入区骨架（与 ChatInput 同高） -->
      <div class="skel-input">
        <div class="skel-line skel-line-input" />
        <div class="skel-line skel-line-send-btn" />
      </div>
    </main>
  </div>
</template>

<style scoped>
/* =========================================================================
 * 根容器：与 AppLayout.app-shell 完全同构
 * ========================================================================= */
.skeleton-shell {
  display: flex;
  height: 100vh;
  overflow: hidden;
  background: var(--ip-color-bg-primary);
}

/* =========================================================================
 * 通用 shimmer 扫光效果
 *
 * 实现策略：
 *   - 占位元素背景色用 --ip-color-bg-tertiary
 *   - ::after 伪元素叠加一个渐变蒙版 + 动画 translateX
 *   - prefers-reduced-motion: reduce 时禁用动画（无障碍合规）
 * ========================================================================= */
.skel-line {
  position: relative;
  overflow: hidden;
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-sm, 4px);
}

.skel-line::after {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(
    90deg,
    transparent 0%,
    rgba(255, 255, 255, 0.55) 50%,
    transparent 100%
  );
  transform: translateX(-100%);
  animation: skel-shimmer var(--ip-duration-skeleton, 1800ms)
    cubic-bezier(0.4, 0.0, 0.2, 1) infinite;
}

/* 暗色模式扫光略暗一些（避免过亮） */
[data-theme="dark"] .skel-line::after,
.dark .skel-line::after {
  background: linear-gradient(
    90deg,
    transparent 0%,
    rgba(255, 255, 255, 0.12) 50%,
    transparent 100%
  );
}

@keyframes skel-shimmer {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(100%);
  }
}

@media (prefers-reduced-motion: reduce) {
  .skel-line::after {
    animation: none;
    opacity: 0;
  }
}

/* =========================================================================
 * 行宽 / 尺寸 token（统一管理便于阅读）
 * ========================================================================= */
.skel-line-lg {
  width: 70%;
  height: 18px;
  margin-bottom: 8px;
}
.skel-line-md {
  width: 55%;
  height: 14px;
  margin-bottom: 6px;
}
.skel-line-sm {
  width: 40%;
  height: 12px;
}
.skel-line-xs {
  width: 30%;
  height: 10px;
}
.skel-line-avatar {
  width: 32px;
  height: 32px;
  border-radius: 50%;
}
.skel-line-btn {
  width: 80px;
  height: 32px;
  border-radius: var(--ip-radius-md, 6px);
}

/* =========================================================================
 * 侧边栏骨架
 * ========================================================================= */
.skeleton-sidebar {
  width: 260px;
  flex-shrink: 0;
  border-right: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-primary);
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 16px 12px;
  gap: 16px;
}

.skel-block {
  display: flex;
  flex-direction: column;
}

.skel-project-selector {
  padding: 8px 10px;
  border-radius: var(--ip-radius-md, 6px);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
}

.skel-new-chat {
  padding: 10px;
  border-radius: var(--ip-radius-md, 6px);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
}

/* 会话列表占位：flex:1 让它撑满中间 */
.skel-conversation-list {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 8px;
  overflow: hidden;
}

.skel-conversation-item {
  padding: 10px 12px;
  border-radius: var(--ip-radius-md, 6px);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  display: flex;
  flex-direction: column;
}

.skel-sidebar-footer {
  margin-top: auto;
  padding: 8px 0;
  border-top: 1px solid var(--ip-color-border-default);
}

/* =========================================================================
 * 主内容区骨架
 * ========================================================================= */
.skeleton-main {
  flex: 1 1 auto;
  min-width: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--ip-color-bg-primary);
}

/* 顶部 header：与 ChatHeader 同高 48px */
.skel-header {
  height: 48px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--ip-color-border-default);
  padding: 0 20px;
  display: flex;
  align-items: center;
  gap: 12px;
  background: var(--ip-color-bg-primary);
}

.skel-header-text {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.skel-header .skel-line-avatar {
  flex-shrink: 0;
}

/* 消息列表：flex:1 让它撑满中间 */
.skel-message-list {
  flex: 1 1 auto;
  overflow: hidden;
  padding: 24px 20px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.skel-message {
  display: flex;
  width: 100%;
}

/* user 消息靠右 */
.skel-message-user {
  justify-content: flex-end;
}

.skel-message-ai {
  justify-content: flex-start;
}

.skel-bubble {
  max-width: 65%;
  padding: 12px 16px;
  border-radius: var(--ip-radius-lg, 12px);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  display: flex;
  flex-direction: column;
}

.skel-message-user .skel-bubble {
  background: var(--ip-color-bg-user-bubble, var(--ip-primary-600, #2563eb));
  border-color: transparent;
  opacity: 0.85;
}

.skel-message-user .skel-line {
  background-color: rgba(255, 255, 255, 0.3);
}

.skel-message-user .skel-line::after {
  background: linear-gradient(
    90deg,
    transparent 0%,
    rgba(255, 255, 255, 0.45) 50%,
    transparent 100%
  );
}

.skel-line-bubble-lg {
  width: 90%;
  height: 14px;
  margin-bottom: 6px;
}
.skel-line-bubble-md {
  width: 70%;
  height: 14px;
  margin-bottom: 6px;
}
.skel-line-bubble-sm {
  width: 40%;
  height: 14px;
}

/* 底部输入区：与 ChatInput 同高（通常 ~80px，含 padding） */
.skel-input {
  flex-shrink: 0;
  border-top: 1px solid var(--ip-color-border-default);
  padding: 12px 20px 16px;
  display: flex;
  align-items: flex-end;
  gap: 12px;
  background: var(--ip-color-bg-primary);
}

.skel-line-input {
  flex: 1 1 auto;
  height: 44px;
  border-radius: var(--ip-radius-md, 6px);
}

.skel-line-send-btn {
  width: 44px;
  height: 44px;
  border-radius: var(--ip-radius-md, 6px);
  flex-shrink: 0;
}
</style>