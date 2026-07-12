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
  padding: 48px var(--ip-spacing-6);
  text-align: center;
  background: var(--ip-color-bg-primary);
}

.hint-icon {
  width: 56px;
  height: 56px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--ip-radius-full);
  background: var(--ip-primary-50);
  margin-bottom: var(--ip-spacing-4);
}

.hint-icon-text {
  font-size: 28px;
  color: var(--ip-primary-500);
  font-weight: 300;
}

.hint-title {
  margin: 0 0 var(--ip-spacing-2);
  font-size: var(--ip-text-body-lg-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.hint-desc {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  max-width: 320px;
}
</style>
