// usePhaseClock — 相位行计时 tick（2026-10-26 生成中状态渲染批 3）
//
// useThinkingTimer 模式泛化：sending 期间 200ms setInterval 刷 now →
// computed 输出「第 N 轮 · Xs」进度数据（formatThinkingMs 真相源）。
// KeepAlive 协同（onActivated 恢复 / onDeactivated 停表）同款。
//
// 时长口径 v1：相位进入时刻起算（now - phase.since，纯前端零依赖）；
// 轮号 = 桶.round（chat:round-state 权威）。回合累计 elapsedMs 保留在桶
// 供后续打磨（等待用户相位不显时长——用户不该被催）。
import { ref, computed, watch, onActivated, onDeactivated, onUnmounted } from "vue";
import { useChatStore } from "../stores/chat";
import { formatThinkingMs } from "../utils/format";

export function usePhaseClock() {
  const chat = useChatStore();
  const now = ref(Date.now());
  let timer: ReturnType<typeof setInterval> | null = null;

  const tick = () => { now.value = Date.now(); };
  const start = () => {
    if (timer) return;
    timer = setInterval(tick, 200);
    tick();
  };
  const stop = () => {
    if (timer) { clearInterval(timer); timer = null; }
  };

  watch(
    () => chat.sending,
    (on) => { if (on) start(); else stop(); },
    { immediate: true },
  );

  /** 相位行进度数据：轮号（round-state 权威）+ 相位已持续时长。 */
  const phaseProgress = computed(() => {
    const s = chat.streamOf(chat.activeConvId ?? "");
    if (!s || !s.sending) return null;
    return {
      round: s.round > 0 ? s.round : null,
      elapsedLabel: formatThinkingMs(now.value - s.phase.since),
    };
  });

  onActivated(start);
  onDeactivated(stop);
  onUnmounted(stop);

  return { phaseProgress };
}
