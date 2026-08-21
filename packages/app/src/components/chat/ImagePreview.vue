<!--
  ImagePreview — 全屏图片预览（单图 / 多图翻页）

  功能：
  - 全屏遮罩 + 居中大图
  - 多图：左右箭头 / ←→ 键翻页，顶部 "n / N" 计数
  - 滚轮缩放、双击切换 1x ⇄ 放大
  - Esc 或点遮罩关闭
  - Teleport 到 body，脱离消息流层级

  Props: images（{data,mediaType}[]）、startIndex
  Emits: close
-->
<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";

const props = defineProps<{
  images: { data: string; mediaType: string }[];
  startIndex: number;
}>();
const emit = defineEmits<{ close: [] }>();

const index = ref(props.startIndex);
const scale = ref(1);

function srcOf(i: number): string {
  const img = props.images[i];
  return `data:${img.mediaType};base64,${img.data}`;
}
function prev(e?: Event) {
  e?.stopPropagation();
  if (index.value > 0) { index.value--; scale.value = 1; }
}
function next(e?: Event) {
  e?.stopPropagation();
  if (index.value < props.images.length - 1) { index.value++; scale.value = 1; }
}
function onWheel(e: WheelEvent) {
  scale.value = Math.min(5, Math.max(0.2, scale.value + (e.deltaY < 0 ? 0.15 : -0.15)));
}
function toggleZoom(e: Event) {
  e.stopPropagation();
  scale.value = scale.value === 1 ? 2.5 : 1;
}
function onKey(e: KeyboardEvent) {
  if (e.key === "ArrowLeft") prev();
  else if (e.key === "ArrowRight") next();
  else if (e.key === "Escape") emit("close");
}
onMounted(() => {
  window.addEventListener("keydown", onKey);
  document.body.style.overflow = "hidden";
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  document.body.style.overflow = "";
});
</script>

<template>
  <Teleport to="body">
    <div class="image-preview-mask" @click="emit('close')" @wheel.prevent="onWheel">
      <div class="preview-counter">{{ index + 1 }} / {{ images.length }}</div>
      <button v-if="images.length > 1 && index > 0" class="preview-nav prev" title="上一张 (←)" @click="prev">‹</button>
      <img
        :src="srcOf(index)"
        class="preview-img"
        :style="{ transform: `scale(${scale})` }"
        draggable="false"
        @click="toggleZoom"
      />
      <button v-if="images.length > 1 && index < images.length - 1" class="preview-nav next" title="下一张 (→)" @click="next">›</button>
      <div class="preview-hint">滚轮/双击缩放 · Esc 关闭</div>
    </div>
  </Teleport>
</template>

<style scoped>
.image-preview-mask {
  position: fixed; inset: 0; z-index: var(--ip-z-modal-overlay);
  background: rgba(0, 0, 0, 0.92);
  display: flex; align-items: center; justify-content: center;
  cursor: zoom-out;
}
.preview-img {
  max-width: 90vw; max-height: 86vh;
  object-fit: contain;
  border-radius: 4px;
  transition: transform 0.15s ease-out;
  cursor: zoom-in;
  user-select: none;
  box-shadow: 0 8px 40px rgba(0, 0, 0, 0.6);
}
.preview-counter {
  position: absolute; top: 20px; left: 50%; transform: translateX(-50%);
  color: rgba(255, 255, 255, 0.85); font-size: 14px;
  background: rgba(0, 0, 0, 0.4); padding: 4px 14px; border-radius: 999px;
}
.preview-nav {
  position: absolute; top: 50%; transform: translateY(-50%);
  width: 48px; height: 48px; border: none; border-radius: 50%;
  background: rgba(255, 255, 255, 0.15); color: #fff;
  font-size: 28px; line-height: 1; cursor: pointer;
  display: flex; align-items: center; justify-content: center;
  transition: background 0.15s;
}
.preview-nav:hover { background: rgba(255, 255, 255, 0.3); }
.preview-nav.prev { left: 24px; }
.preview-nav.next { right: 24px; }
.preview-hint {
  position: absolute; bottom: 20px; left: 50%; transform: translateX(-50%);
  color: rgba(255, 255, 255, 0.5); font-size: 12px;
}
</style>
