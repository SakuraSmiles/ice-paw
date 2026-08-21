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
    scale.value = 1;        // 新图：cover 基线
    offset.value = { x: 0.5, y: 0.5 }; // 居中
  }
}

// 时序保险：换 src 时若图已解码（load 早于挂载/同 URL 复用），下一帧直读尺寸兜底
import { watch as _w, nextTick as _nt } from "vue";
_w(imgSrc, async () => {
  await _nt();
  if (imgEl.value && imgEl.value.complete && imgW.value === 0) {
    imgW.value = imgEl.value.naturalWidth;
    imgH.value = imgEl.value.naturalHeight;
  }
});

// ---- 图片变换模型（用户拍板 2026-08-21）：固定取景框（舞台中央）+ 图片平移/缩放 ----
// offset = 图片中心在舞台坐标的归一化位置（拖动改）；scale = 缩放系数（1 = cover 基线）。
const offset = ref<CropOffset>({ x: 0.5, y: 0.5 });
const scale = ref(1);
const stageRef = ref<HTMLElement | null>(null);

/** 取景框边长（固定：舞台短边与 200 的较小者）。 */
function frameSize(): number {
  const st = stageRef.value?.getBoundingClientRect();
  const short = st ? Math.min(st.width, st.height) : 240;
  return Math.round(Math.min(short, 200));
}

const FRAME_MIN_SCALE = 1;
const FRAME_MAX_SCALE = 6;
function clampScale(v: number): number {
  return Math.min(Math.max(v, FRAME_MIN_SCALE), FRAME_MAX_SCALE);
}

/** 缩放步进 15%（zoomIn=放大看更细）。 */
function zoom(dir: 1 | -1) {
  scale.value = clampScale(+(scale.value * (1 + dir * 0.15)).toFixed(3));
}

/** 图片显示短边 = 取景框边长 × scale。 */
function imgDisplayShort(): number {
  return Math.round(frameSize() * scale.value);
}

/** 图片完整显示宽高（按原始宽高比从短边推）。 */
function imgDisplay(): { w: number; h: number } {
  if (imgW.value === 0 || imgH.value === 0) return { w: 0, h: 0 };
  const short = imgDisplayShort();
  return imgW.value <= imgH.value
    ? { w: short, h: Math.round((short * imgH.value) / imgW.value) }
    : { w: Math.round((short * imgW.value) / imgH.value), h: short };
}

/** 图片左上位置：offset 中心换算 + 钳制（图片四边恒盖住取景框）。 */
const imgPos = computed(() => {
  const st = stageRef.value?.getBoundingClientRect();
  const d = imgDisplay();
  if (!st || d.w === 0) return { left: 0, top: 0 };
  const cxMin = frameSize() / 2;
  const cxMax = st.width - frameSize() / 2;
  const cyMin = frameSize() / 2;
  const cyMax = st.height - frameSize() / 2;
  const left = Math.min(
    Math.max(offset.value.x * st.width - d.w / 2, cxMin - d.w / 2),
    cxMax - d.w / 2,
  );
  const top = Math.min(
    Math.max(offset.value.y * st.height - d.h / 2, cyMin - d.h / 2),
    cyMax - d.h / 2,
  );
  return { left, top };
});

// 滚轮/捏合缩放（+/- 按钮同调 zoom；ctrlKey 捏合在 WKWebView 映射为 wheel）
function onWheel(e: WheelEvent) {
  if (!ready.value) return;
  e.preventDefault();
  zoom(e.deltaY < 0 ? 1 : -1);
}

// ---- 拖动（pointer events：鼠标+触摸统一）----
let dragging = false;

function onPointerDown(e: PointerEvent) {
  dragging = true;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
}

function onPointerMove(e: PointerEvent) {
  if (!dragging || !stageRef.value) return;
  const st = stageRef.value.getBoundingClientRect();
  const d = imgDisplay();
  if (d.w === 0) return;
  const cx = e.clientX - st.left;
  const cy = e.clientY - st.top;
  // 指针位置 = 图片中心；按 imgPos 同款钳制（图片盖满取景框）
  const cxMin = frameSize() / 2;
  const cxMax = st.width - frameSize() / 2;
  const cyMin = frameSize() / 2;
  const cyMax = st.height - frameSize() / 2;
  const left = Math.min(Math.max(cx - d.w / 2, cxMin - d.w / 2), cxMax - d.w / 2);
  const top = Math.min(Math.max(cy - d.h / 2, cyMin - d.h / 2), cyMax - d.h / 2);
  offset.value = { x: (left + d.w / 2) / st.width, y: (top + d.h / 2) / st.height };
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
  if (!imgEl.value || !stageRef.value || working.value) return;
  working.value = true;
  errMsg.value = "";
  try {
    // 取景框中心在舞台的位置 → 图片显示坐标 → 原图归一化坐标
    const st = stageRef.value.getBoundingClientRect();
    const fx = st.width / 2 - imgPos.value.left;
    const fy = st.height / 2 - imgPos.value.top;
    const d = imgDisplay();
    const cropOffset: CropOffset = d.w > 0 ? {
      x: Math.min(Math.max(fx / d.w, 0), 1),
      y: Math.min(Math.max(fy / d.h, 0), 1),
    } : { x: 0.5, y: 0.5 };
    const data = await compressAvatarImage(imgEl.value, cropOffset);
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
          <span class="ac-sub">拖动定位 · 滚轮/捏合缩放</span>
        </h4>

        <!-- 裁剪舞台：固定取景框（中央）+ 可拖拽/缩放的图片（用户拍板模型） -->
        <div
          v-if="imgSrc"
          ref="stageRef"
          class="ac-stage"
          @pointerdown.prevent="onPointerDown"
          @wheel.prevent="onWheel"
        >
          <!-- 底层：图片（拖动/缩放的变换载体；宽度=显示宽，位置=钳制后左上） -->
          <img
            ref="imgEl"
            :src="imgSrc ?? undefined"
            class="ac-img"
            :style="{ width: imgDisplay().w + 'px', height: imgDisplay().h + 'px', left: imgPos.left + 'px', top: imgPos.top + 'px' }"
            draggable="false"
            @load="onImgLoad"
            @error="errMsg = '图片加载失败，请重新选择'"
          />
          <!-- 顶层：固定取景框（中央白框 + 框外压暗 + 四角刻度） -->
          <div v-if="ready" class="ac-frame" :style="{ width: frameSize() + 'px', height: frameSize() + 'px' }">
            <span class="ac-corner tl" /><span class="ac-corner tr" />
            <span class="ac-corner bl" /><span class="ac-corner br" />
          </div>
        </div>

        <!-- 无图：第一步选图 -->
        <div v-if="!imgSrc" class="ac-empty" @click="fileInput?.click()">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="4"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>
          <span>{{ imgSrc ? "图片加载中…" : "点击选择图片（支持拖入）" }}</span>
        </div>

        <div class="ac-foot">
          <div class="ac-zoom">
            <button type="button" class="ac-zoom-btn" title="缩小" :disabled="scale <= FRAME_MIN_SCALE" @click="zoom(-1)">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="5" y1="12" x2="19" y2="12"/></svg>
            </button>
            <span class="ac-zoom-val">{{ Math.round(scale * 100) }}%</span>
            <button type="button" class="ac-zoom-btn" title="放大" :disabled="scale >= FRAME_MAX_SCALE" @click="zoom(1)">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
            </button>
          </div>
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
/* 图片：绝对定位（变换载体；宽高/位置由内联 style 驱动） */
.ac-img {
  position: absolute;
  object-fit: fill; /* 尺寸由 style 精确指定，禁 contain 的自动适配 */
  cursor: grab;
  max-width: none; /* 覆盖全局 img max-width 限制（可放大超舞台） */
}
.ac-img:active { cursor: grabbing; }

/* 固定取景框：中央白框 + 框外压暗（大投影蒙层）+ 四角刻度 */
.ac-frame {
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  border: 2px solid #fff;
  box-shadow: 0 0 0 9999px rgba(11, 14, 18, 0.5);
  pointer-events: none; /* 框是视觉层，拖的是底下的图 */
}
.ac-corner { position: absolute; width: 12px; height: 12px; border-color: #fff; border-style: solid; }
.ac-corner.tl { left: -2px; top: -2px; border-width: 3px 0 0 3px; }
.ac-corner.tr { right: -2px; top: -2px; border-width: 3px 3px 0 0; }
.ac-corner.bl { left: -2px; bottom: -2px; border-width: 0 0 3px 3px; }
.ac-corner.br { right: -2px; bottom: -2px; border-width: 0 3px 3px 0; }

/* 缩放控件（+/- + 百分比） */
.ac-zoom { display: flex; align-items: center; gap: 6px; }
.ac-zoom-btn {
  width: 24px; height: 24px;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-primary);
  color: var(--ip-color-text-secondary);
  display: flex; align-items: center; justify-content: center;
  cursor: pointer;
}
.ac-zoom-btn:hover:not(:disabled) { color: var(--ip-primary-600); border-color: var(--ip-primary-400); }
.ac-zoom-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.ac-zoom-val { font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); font-family: var(--ip-font-mono); min-width: 38px; text-align: center; }
.ac-file { display: none; }
</style>
