/**
 * liveRenderFrozen — 流式 live 渲染的页面可见性冻结开关（2026-11-04 线程饥饿根治批）。
 *
 * 背景：keep-alive 缓存的页面被 deactivated 后，组件的响应式 effect **仍然活着**——
 * 用户切到设置页时，聊天页的 live 气泡照常每个聚合 tick 全量重解析 markdown +
 * 重建 innerHTML（隐藏渲染），是「生成中切设置页卡顿」的隐形主线程源。
 *
 * 写权在 ChatPage（onActivated/onDeactivated + visibilitychange）；读方
 * MarkdownRenderer 冻结期跳过重渲染、解冻统一补一次。回退安全：默认 false
 * （不冻结），ChatPage 未挂载时行为与旧版一致。
 */
import { ref } from "vue";

export const liveRenderFrozen = ref(false);
