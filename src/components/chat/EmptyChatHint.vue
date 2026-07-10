<script setup lang="ts">
// 聊天区空状态提示
//
// 职责：
//   - 当前会话没有任何消息且非流中时显示
//   - 给出「输入第一条消息开始对话」的引导
//
// props: 无
// emits: 无
</script>

<template>
  <div class="empty-chat-hint">
    <div class="hint-icon" aria-hidden="true">
      <span class="hint-icon-text">⌬</span>
    </div>
    <h3 class="hint-title">开始一段对话</h3>
    <p class="hint-desc">在下方输入框中输入你的第一条消息，回车发送给 AI。</p>
  </div>
</template>

<style scoped>
.empty-chat-hint {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px 24px;
  text-align: center;
  background: var(--chat-bg, #fafafa);
}

.hint-icon {
  width: 56px;
  height: 56px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--hint-icon-bg, rgba(26, 115, 232, 0.08));
  margin-bottom: 16px;
}

.hint-icon-text {
  font-size: 28px;
  color: var(--accent-fg, #1a73e8);
  font-weight: 300;
}

.hint-title {
  margin: 0 0 8px;
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
}

.hint-desc {
  margin: 0;
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-secondary, #888);
  max-width: 320px;
}

@media (prefers-color-scheme: dark) {
  .empty-chat-hint {
    --chat-bg: #181828;
  }
  .hint-icon {
    --hint-icon-bg: rgba(74, 144, 226, 0.16);
  }
  .hint-icon-text {
    --accent-fg: #6ba9e8;
  }
  .hint-title {
    --text-primary: #f0f0f0;
  }
  .hint-desc {
    --text-secondary: #888;
  }
}
</style>
