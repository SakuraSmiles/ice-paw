<script setup lang="ts">
// ScreenFrame.vue — 屏幕共享红边框（批次④ 步骤 2，§4.7）
//
// 透明置顶点击穿透窗（Rust 侧 hud.rs 创建并设穿透），覆盖整个虚拟桌面。
// 本页纯 CSS：3px danger 描边 = 「屏幕正在被共享」的持续可见信号。
// 零交互零权限（不在任何 capability 中）；窗口本体随通道开/关建/毁。
import { onMounted } from "vue";

onMounted(() => {
  // 窗口透明依赖页面背景透明——global.css 的 body 背景会把它漆成不透明。
  // 本窗是独立 WebView，直接改本窗的 body 不影响主窗。
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
});
</script>

<template>
  <div class="screen-frame" aria-hidden="true" />
</template>

<style scoped>
.screen-frame {
  position: fixed;
  inset: 0;
  box-sizing: border-box;
  border: 3px solid var(--ip-danger-base, #b83d3d);
  pointer-events: none;
}
</style>
