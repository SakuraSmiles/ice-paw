// composables/useThinkingTimer.ts
// 思考耗时实时计时：streamingThinking 期间每 200ms tick 一次，供「思考·进行中…」
// 实时显示已经过时间。从 ChatMessages.vue 抽出。
//
// KeepAlive 协同：切走时停表（不可见区间不浪费 CPU），切回且仍在思考则恢复。
// composable 内部注册 onActivated/onDeactivated/onUnmounted，与父组件其它生命周期并行触发。

import { ref, computed, watch, onActivated, onDeactivated, onUnmounted } from "vue";
import { useChatStore } from "../stores/chat";

export function useThinkingTimer() {
  const chat = useChatStore();
  const thinkingNow = ref(Date.now());
  let thinkingTimer: ReturnType<typeof setInterval> | null = null;

  watch(() => chat.streamingThinking, (val) => {
    if (val && !thinkingTimer) {
      thinkingTimer = setInterval(() => { thinkingNow.value = Date.now(); }, 200);
    } else if (!val && thinkingTimer) {
      clearInterval(thinkingTimer);
      thinkingTimer = null;
    }
  });

  const thinkingElapsed = computed(() => {
    const start = chat.thinkingStartTime;
    if (!start) return '';
    const elapsed = Math.floor((thinkingNow.value - start) / 1000);
    if (elapsed < 60) return `${elapsed}s`;
    const m = Math.floor(elapsed / 60);
    const s = elapsed % 60;
    return `${m}m ${s}s`;
  });

  // KeepAlive：切走停表，切回且仍在思考则恢复
  onActivated(() => {
    if (chat.streamingThinking && !thinkingTimer) {
      thinkingNow.value = Date.now();
      thinkingTimer = setInterval(() => { thinkingNow.value = Date.now(); }, 200);
    }
  });
  onDeactivated(() => {
    if (thinkingTimer) { clearInterval(thinkingTimer); thinkingTimer = null; }
  });
  onUnmounted(() => {
    if (thinkingTimer) { clearInterval(thinkingTimer); thinkingTimer = null; }
  });

  return { thinkingElapsed };
}
