<!--
  AvatarCropper — 头像裁剪器弹层（AvatarField 内部使用）

  交互（用户拍板 2026-08-21）：方形视窗在原图上拖动定位（不做缩放/旋转——
  头像 90% 场景是"框住哪块"）；蒙层外半透明显示全景。
  入口职责：无原图时第一步是选图；有原图直接定位（重新选图按钮换图）。
  Esc 走 useEscapeStack（与图片预览器互斥，只关栈顶）。

  Props: source?: string | null（原图 dataURL——AvatarField 已读入的待裁剪图）
  Emits:
    - confirm(dataURL: string)：确认 → 压缩产物
    - cancel：取消（含 Esc）
-->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useEscapeStack } from "../../composables/useEscapeStack";
import {
  compressAvatarImage,
  type CropOffset,
} from "../../utils/avatar";

const props = defineProps<{ source?: string | null }>();
const emit = defineEmits<{ confirm: [data: string]; cancel: [] }>();

useEscapeStack(() => emit("cancel"));

// ---- 原图（source 或内部新选）----
const fileInput = ref<HTMLInputElement | null>(null);
const imgEl = ref<HTMLImageElement | null>(null);
const imgSrc = ref<string | null>(props.source ?? null);
const imgW = ref(0);
const imgH = ref(0);
const ready = computed(() => !!imgSrc.value && imgW.value > 0);

function onFile(e: Event) {
  const f = (e.target as HTMLInputElement).files?.[0];
  if (f) void loadFile(f);
  (e.target as HTMLInputElement).value = ""; // 同文件可重选
}

async function loadFile(f: File) {
  const url = URL.createObjectURL(f);
  imgSrc.value = url;
  offset.value = { x: 0.5, y: 0.5 }; // 新图回中心
}

function onImgLoad() {
  if (imgEl.value) {
    imgW.value = imgEl.value.naturalWidth;
    imgH.value = imgEl.value.naturalHeight;
  }
}

// ---- 视窗定位（归一化中心点，渲染时换算像素）----
const offset = ref<CropOffset>({ x: 0.5, y: 0.5 });
const stageRef = ref<HTMLElement | null>(null);

// 视窗边长（px，相对原图）与 stage 显示尺寸的比例换算：
// 显示层用 <img> 自然布局（max 约束），直接以 stage 内像素工作——
// 视窗 = 原图短边的显示比例。简化：视窗占 stage 短边的 72%（留呼吸）。
const winSize = computed(() => {
  if (!stageRef.value) return 150;
  const r = stageRef.value.getBoundingClientRect();
  return Math.round(Math.min(r.width, r.height) * 0.72);
});

/** 视窗左上像素位（stage 坐标系）——由 offset 中心点换算，钳制在 stage 内。 */
const winPos = computed(() => {
  const s = stageRef.value?.getBoundingClientRect();
  if (!s) return { left: 0, top: 0 };
  const w = winSize.value;
  // offset 是相对【原图】的比例；stage 即原图显示区（图铺满 stage），
  // 中心点直接按 stage 尺寸换算，短边方向天然贴边可用。
  const left = Math.min(
    Math.max(offset.value.x * s.width - w / 2, 0),
    Math.max(s.width - w, 0),
  );
  const top = Math.min(
    Math.max(offset.value.y * s.height - w / 2, 0),
    Math.max(s.height - w, 0),
  );
  return { left, top };
});

// ---- 拖动（pointer events：鼠标+触摸统一）----
let dragging = false;

function onPointerDown(e: PointerEvent) {
  dragging = true;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
}

function onPointerMove(e: PointerEvent) {
  if (!dragging || !stageRef.value) return;
  const s = stageRef.value.getBoundingClientRect();
  const w = winSize.value;
  // 指针位置 → 视窗中心（钳制），再反解归一化 offset
  const cx = e.clientX - s.left;
  const cy = e.clientY - s.top;
  const left = Math.min(Math.max(cx - w / 2, 0), Math.max(s.width - w, 0));
  const top = Math.min(Math.max(cy - w / 2, 0), Math.max(s.height - w, 0));
  offset.value = {
    x: s.width > w ? (left + w / 2) / s.width : 0.5,
    y: s.height > w ? (top + w / 2) / s.height : 0.5,
  };
}

function onPointerUp() {
  dragging = false;
}

onMounted(() => {
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
});
onUnmounted(() => {
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
});

// ---- 确认：原图 element + offset → 压缩管道 ----
const working = ref(false);
const errMsg = ref("");

async function onConfirm() {
  if (!imgEl.value || working.value) return;
  working.value = true;
  errMsg.value = "";
  try {
    const data = await compressAvatarImage(imgEl.value, offset.value);
    emit("confirm", data);
  } catch (e) {
    errMsg.value = e instanceof Error ? e.message : "图片处理失败";
  } finally {
    working.value = false;
  }
}
</script>

<template>
  <Teleport to="body">
    <div class="ac-backdrop" @click.self="emit('cancel')">
      <div class="ac-modal" role="dialog" aria-label="调整头像">
        <h4 class="ac-title">
          调整头像
          <span class="ac-sub">拖动方框选择保留区域</span>
        </h4>

        <!-- 裁剪舞台 -->
        <div
          v-if="ready"
          ref="stageRef"
          class="ac-stage"
          @pointerdown.prevent="onPointerDown"
        >
          <img
            ref="imgEl"
            :src="imgSrc ?? undefined"
            class="ac-img"
            draggable="false"
            @load="onImgLoad"
          />
          <!-- 全景蒙层：视窗外压暗（box-shadow 大投影技巧） -->
          <div
            class="ac-win"
            :style="{ width: winSize + 'px', height: winSize + 'px', left: winPos.left + 'px', top: winPos.top + 'px' }"
          >
            <img :src="imgSrc ?? undefined" class="ac-win-img" :style="{ width: stageRef?.clientWidth + 'px' }" draggable="false" />
            <span class="ac-grip" aria-hidden="true" />
          </div>
        </div>

        <!-- 无图：第一步选图 -->
        <div v-else class="ac-empty" @click="fileInput?.click()">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="4"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>
          <span>{{ imgSrc ? "图片加载中…" : "点击选择图片（支持拖入）" }}</span>
        </div>

        <div class="ac-foot">
          <span class="ac-hint">{{ errMsg || (ready ? winSize + "×" + winSize : "") }}</span>
          <div class="ac-actions">
            <button type="button" class="ac-btn ghost" @click="fileInput?.click()">重新选图</button>
            <button type="button" class="ac-btn ghost" @click="emit('cancel')">取消</button>
            <button type="button" class="ac-btn primary" :disabled="!ready || working" @click="onConfirm">
              {{ working ? "处理中…" : "确认" }}
            </button>
          </div>
        </div>

        <input ref="fileInput" type="file" accept="image/*" class="ac-file" @change="onFile" />
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.ac-backdrop {
  position: fixed;
  inset: 0;
  z-index: var(--ip-z-modal-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--ip-color-bg-overlay);
}
.ac-modal {
  width: 420px;
  max-width: calc(100vw - 48px);
  padding: 18px;
  border-radius: var(--ip-radius-lg);
  background: var(--ip-color-bg-primary);
  box-shadow: var(--ip-shadow-lg);
}
.ac-title { font-size: var(--ip-text-body-size); font-weight: 600; color: var(--ip-color-text-primary); margin: 0 0 12px; }
.ac-sub { font-weight: 400; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); margin-left: 8px; }

.ac-stage {
  position: relative;
  height: 240px;
  border-radius: var(--ip-radius-md);
  overflow: hidden;
  background: var(--ip-gray-950);
  user-select: none;
  touch-action: none;
}
.ac-img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; }
/* 视窗：白框 + 大投影做蒙层；内嵌同图裁剪平移对齐（win-img 反向偏移由 left/top margin 实现） */
.ac-win {
  position: absolute;
  border: 2.5px solid #fff;
  border-radius: var(--ip-radius-sm);
  box-shadow: 0 0 0 9999px rgba(11, 14, 18, 0.45);
  overflow: hidden;
  cursor: grab;
}
.ac-win:active { cursor: grabbing; }
.ac-win-img { position: absolute; object-fit: contain; pointer-events: none; }
.ac-grip {
  position: absolute;
  right: 3px;
  bottom: 3px;
  width: 16px;
  height: 16px;
  background: rgba(255, 255, 255, 0.85);
  border-radius: 0 0 4px 0;
  clip-path: polygon(100% 0, 100% 100%, 0 100%);
}

.ac-empty {
  height: 160px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 1.5px dashed var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-body-sm-size);
  cursor: pointer;
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ac-empty:hover { border-color: var(--ip-primary-400); color: var(--ip-primary-600); }

.ac-foot { display: flex; align-items: center; justify-content: space-between; margin-top: 12px; }
.ac-hint { font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); font-family: var(--ip-font-mono); }
.ac-actions { display: flex; gap: 8px; }
.ac-btn {
  height: 30px;
  padding: 0 16px;
  border: none;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size);
  font-weight: 500;
  cursor: pointer;
}
.ac-btn.primary { background: var(--ip-primary-500); color: #fff; }
.ac-btn.primary:hover:not(:disabled) { background: var(--ip-primary-600); }
.ac-btn.primary:disabled { opacity: 0.5; cursor: not-allowed; }
.ac-btn.ghost { background: transparent; color: var(--ip-color-text-secondary); }
.ac-btn.ghost:hover { background: var(--ip-color-bg-tertiary); }
.ac-file { display: none; }
</style>
