// 屏幕共享通道状态（computer-use 批次④ 步骤 1）
//
// 通道 = 授权与可见性的单位（非物理管道）：Active 且本会话附着时，computer-use
// 家族工具的逐次 Confirm 被短路为 Allow（后端 channel::short_circuit）。本 store
// 只持有一份通道态镜像（单一全量事件 screen:channel-state 驱动 + 启动初拉），
// 开/关动作转发 bridge；授权语义全在后端，前端不重复判断。
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { ScreenChannelState } from "../types";
import { bridge } from "../api/bridge";

const OFF_STATE: ScreenChannelState = {
  status: "off",
  paused: false,
  opened_at: null,
  hud_monitor: 0,
  attached: [],
  holder: null,
  queue: [],
  human_active: false,
  writing: false,
  screenshot_count: 0,
};

export const useScreenChannelStore = defineStore("screenChannel", () => {
  const state = ref<ScreenChannelState>({ ...OFF_STATE });
  /** 开关动作飞行中（防连点：开/关都是即时动作，重复触发无意义） */
  const busy = ref(false);
  let initialized = false;
  const noop = () => {};

  const isOn = computed(() => state.value.status === "active");

  function isAttached(convId: string | null | undefined): boolean {
    if (!convId || state.value.status !== "active") return false;
    return state.value.attached.some((c) => c.conv_id === convId);
  }

  /** App.vue 启动接线：初拉通道态 + 订阅全量事件（进程级单例，重启即 Off，
   *  无持久化）。幂等守卫防 HMR/重复挂载双订阅。返回拆卸函数。 */
  async function init(): Promise<() => void> {
    if (initialized) return noop;
    initialized = true;
    const unlisten = await listen<ScreenChannelState>("screen:channel-state", (e) => {
      state.value = e.payload;
    });
    try {
      state.value = await bridge.screen.getChannelState();
    } catch (e) {
      // 初拉失败不致命：进程刚启动通道必为 Off，镜像默认值即真相
      console.error("拉取屏幕通道状态失败:", e);
    }
    return unlisten;
  }

  /** 开启通道 / 把会话加入共享（已开时后端 open = 仅附着）。抛错交调用方处理。 */
  async function openFrom(convId: string): Promise<void> {
    if (busy.value) return;
    busy.value = true;
    try {
      state.value = await bridge.screen.openChannel(convId);
    } finally {
      busy.value = false;
    }
  }

  /** 关闭通道（全部附着会话清空；幂等）。抛错交调用方处理。 */
  async function stop(): Promise<void> {
    if (busy.value) return;
    busy.value = true;
    try {
      state.value = await bridge.screen.stopChannel();
    } finally {
      busy.value = false;
    }
  }

  /** 暂停/恢复（步骤 3 播放器语义：读写 gate 挂起/唤醒，通道与授权保持）。
   *  Off 状态后端幂等无操作。抛错交调用方处理。 */
  async function setPaused(paused: boolean): Promise<void> {
    if (busy.value) return;
    busy.value = true;
    try {
      state.value = paused
        ? await bridge.screen.pauseChannel()
        : await bridge.screen.resumeChannel();
    } finally {
      busy.value = false;
    }
  }

  /** 手动授予写令牌（HUD 队列块「授予」）。抛错交调用方处理。 */
  async function grantTo(convId: string): Promise<void> {
    if (busy.value) return;
    busy.value = true;
    try {
      state.value = await bridge.screen.grantChannel(convId);
    } finally {
      busy.value = false;
    }
  }

  /** HUD 轮询补真相（步骤 2）：human_active/writing 无事件源变化（时间戳/
   *  计数翻转不广播），HUD 页 1s 轮询调用本方法刷新全量态。 */
  async function refresh(): Promise<void> {
    try {
      state.value = await bridge.screen.getChannelState();
    } catch {
      // 轮询失败不致命：下一拍重试（事件通道仍在）
    }
  }

  return { state, busy, isOn, isAttached, init, openFrom, stop, setPaused, grantTo, refresh };
});
