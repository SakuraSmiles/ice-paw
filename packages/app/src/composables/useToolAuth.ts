// useToolAuth — 工具授权请求的全局状态（A2-3）
//
// 职责：
//   1. 订阅 Rust 侧 `chat:tool-auth-request` 事件，把待处理请求入队
//   2. 维护「待响应」队列（同一会话内可能连续触发多个授权弹窗）
//   3. 提供 `respond(requestId, allowed)` action，emit `chat:tool-auth-response`
//   4. 提供 `pendingRequest` getter：始终返回队首请求（弹窗展示用）
//
// 设计要点：
//   - 单例状态：通过模块级 `state` 变量在多次 `useToolAuth()` 调用间共享
//   - 与 chat store 解耦：单独一个 composable，避免 chat store 膨胀
//   - 与 `chatStore.setupListeners()` 一样在 ChatPage onMounted 时初始化一次
//
// 取消/超时处理：
//   - 若 Rust 侧超时（30 分钟），oneshot receiver 会被 drop，
//     前端不会收到响应，前端弹窗一直挂起 —— 这是边界情况。
//     后续若用户切换会话或刷新页面，store 重置会清理挂起项。
//   - 用户主动 `stop_generation` 时，Rust 也会清理 receiver，
//     前端弹窗可监听流式停止事件后自动关闭（见 ToolAuthDialog 实现）。

import { computed, ref } from "vue";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ToolAuthRequestPayload, ToolAuthResponse } from "../types";

// ============================================================================
// 模块级单例状态
// ============================================================================

/** 待响应的授权请求队列（FIFO） */
const pendingQueue = ref<ToolAuthRequestPayload[]>([]);

/** listen 返回的 unlisten 函数（模块级，避免 setupListeners 多次调用泄漏） */
let unlisten: UnlistenFn | null = null;

/** listen 是否已注册（幂等保护） */
let listenersRegistered = false;

// ============================================================================
// composable
// ============================================================================

export function useToolAuth() {
  /**
   * 当前展示用的请求（队列首项）。
   * ToolAuthDialog v-model:open 绑定此值：
   *   - undefined → 关闭
   *   - 有值 → 打开并展示内容
   */
  const pendingRequest = computed<ToolAuthRequestPayload | null>(() => {
    return pendingQueue.value.length > 0 ? pendingQueue.value[0] ?? null : null;
  });

  /**
   * 队列长度（仅供调试 / 测试用）
   */
  const queueLength = computed<number>(() => pendingQueue.value.length);

  /**
   * 注册 `chat:tool-auth-request` 监听器。幂等。
   * 应在 ChatPage onMounted 时调用一次。
   */
  async function setupAuthListener(): Promise<void> {
    if (listenersRegistered) return;
    listenersRegistered = true;

    unlisten = await listen<ToolAuthRequestPayload>(
      "chat:tool-auth-request",
      (e) => {
        const req = e.payload;
        // 去重：相同 request_id 不重复入队
        if (pendingQueue.value.some((r) => r.request_id === req.request_id)) {
          return;
        }
        pendingQueue.value = [...pendingQueue.value, req];
      },
    );
  }

  /**
   * 注销监听器。ChatPage onUnmounted 调用。
   */
  function teardownAuthListener(): void {
    if (unlisten) {
      try {
        unlisten();
      } catch {
        // ignore
      }
      unlisten = null;
    }
    pendingQueue.value = [];
    listenersRegistered = false;
  }

  /**
   * 响应授权请求：emit `chat:tool-auth-response` 给 Rust 侧，
   * 同时从队列中移除该请求。
   *
   * @param requestId 要响应的请求 ID（与 pendingRequest.request_id 对应）
   * @param allowed   true = 允许；false = 拒绝
   */
  async function respond(requestId: string, allowed: boolean): Promise<void> {
    const payload: ToolAuthResponse = { request_id: requestId, allowed };
    try {
      await emit("chat:tool-auth-response", payload);
    } catch (err) {
      // emit 失败（例如 Tauri IPC 异常）：本地清理队列避免卡死
      console.warn("[useToolAuth] emit 授权响应失败", err);
    } finally {
      pendingQueue.value = pendingQueue.value.filter(
        (r) => r.request_id !== requestId,
      );
    }
  }

  /**
   * 强制清空队列（用于 chat:done / chat:error 等流式终止事件后的兜底）。
   * 注意：此动作不会 emit 任何响应给 Rust，被丢弃的 request_id 会在 Rust
   * oneshot 端因 receiver drop 而被自动清理。
   */
  function clearAll(): void {
    pendingQueue.value = [];
  }

  return {
    pendingRequest,
    queueLength,
    setupAuthListener,
    teardownAuthListener,
    respond,
    clearAll,
  };
}