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

  return { state, busy, isOn, isAttached, init, openFrom, stop };
});
