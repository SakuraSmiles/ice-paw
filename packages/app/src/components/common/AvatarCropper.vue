<!--
  AvatarCropper — 头像裁剪器弹层（AvatarField 内部使用）

  2026-08-21 v3：弃自研拖拽/缩放数学（真机连续暴露钳制/时序 bug），换
  vue-cropper（若依/RuoYi 平台头像上传同款成熟库，生产验证充分）。
  模型即用户拍板形态：fixedBox=true 固定居中取景框，拖拽移动图片、
  滚轮/按钮缩放；库内处理全部指针/钳制/惯性细节。
  输出：getCropBlob → dataURL（png 输出保透明 alpha）。

  Props: source?: string | null（原图 dataURL/objectURL）
  Emits: confirm(dataURL) / cancel
-->
<script setup lang="ts">
import { ref } from "vue";
import { VueCropper } from "vue-cropper";
import "vue-cropper/dist/index.css";
import { useEscapeStack } from "../../composables/useEscapeStack";
import { AVATAR_MAX_SRC_BYTES } from "../../utils/avatar";

const props = defineProps<{ source?: string | null }>();
const emit = defineEmits<{ confirm: [data: string]; cancel: [] }>();

useEscapeStack(() => emit("cancel"));

// vue-cropper 未导出实例类型；组件实例仅用三个已知方法，窄接口声明替代 any
interface CropperInstance {
  getCropBlob(cb: (blob: Blob | null) => void): void;
  changeScale(dir: number): void;
}
const cropperRef = ref<CropperInstance | null>(null);
const fileInput = ref<HTMLInputElement | null>(null);
const imgSrc = ref<string>(props.source ?? "");
const working = ref(false);
const errMsg = ref("");

function onFile(e: Event) {
  const f = (e.target as HTMLInputElement).files?.[0];
  (e.target as HTMLInputElement).value = "";
  if (!f) return;
  if (f.size > AVATAR_MAX_SRC_BYTES) {
    errMsg.value = `图片过大（${(f.size / 1024 / 1024).toFixed(1)}MB），请选择 10MB 以内的图片`;
    return;
  }
  errMsg.value = "";
  imgSrc.value = URL.createObjectURL(f);
}

/** 确认：库实时裁剪 → blob → dataURL（png 保透明）。 */
function onConfirm() {
  const c: CropperInstance | null = cropperRef.value;
  if (!c || working.value) return;
  working.value = true;
  c.getCropBlob((blob: Blob | null) => {
    working.value = false;
    if (!blob) {
      errMsg.value = "裁剪失败，请重试";
      return;
    }
    const reader = new FileReader();
    reader.onload = () => emit("confirm", reader.result as string);
    reader.onerror = () => {
      errMsg.value = "读取裁剪结果失败";
    };
    reader.readAsDataURL(blob);
  });
}

/** 缩放按钮（vue-cropper 内建 changeScale，正=放大）。 */
function zoom(dir: 1 | -1) {
  cropperRef.value?.changeScale(dir);
}
</script>

<template>
  <Teleport to="body">
    <div class="ac-backdrop" @click.self="emit('cancel')">
      <div class="ac-modal" role="dialog" aria-label="调整头像">
        <h4 class="ac-title">
          调整头像
          <span class="ac-sub">拖动图片定位 · 滚轮/按钮缩放</span>
        </h4>

        <!-- 裁剪区：vue-cropper 固定框模型（fixedBox + canMove + canScale）。
             fixed + fixedNumber=[1,1] 锁取景框恒正方形——autoCrop 初值虽是 200×200，
             但源图小于取景框时库内钳制会按维度独立收缩（宽被压、高保持 → 瘦高框
             → 瘦高头像，2026-08-22 真机发现），锁比例后钳制也保持 1:1 -->
        <div v-if="imgSrc" class="ac-crop-wrap">
          <VueCropper
            ref="cropperRef"
            class="ac-cropper"
            :img="imgSrc"
            :auto-crop="true"
            :auto-crop-width="200"
            :auto-crop-height="200"
            :fixed="true"
            :fixed-number="[1, 1]"
            :fixed-box="true"
            :can-move="true"
            :can-move-box="false"
            :center-box="true"
            :output-size="0.9"
            output-type="png"
            :info="false"
            :full="false"
            :can-scale="true"
            :high="false"
          />
        </div>

        <!-- 无图：第一步选图 -->
        <div v-else class="ac-empty" @click="fileInput?.click()">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="4"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>
          <span>点击选择图片（支持拖入）</span>
        </div>

        <div class="ac-foot">
          <div class="ac-zoom">
            <button type="button" class="ac-zoom-btn" title="缩小" @click="zoom(-1)">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="5" y1="12" x2="19" y2="12"/></svg>
            </button>
            <button type="button" class="ac-zoom-btn" title="放大" @click="zoom(1)">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
            </button>
            <span v-if="errMsg" class="ac-err">{{ errMsg }}</span>
          </div>
          <div class="ac-actions">
            <button type="button" class="ac-btn ghost" @click="fileInput?.click()">重新选图</button>
            <button type="button" class="ac-btn ghost" @click="emit('cancel')">取消</button>
            <button type="button" class="ac-btn primary" :disabled="!imgSrc || working" @click="onConfirm">
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
  width: 440px;
  max-width: calc(100vw - 48px);
  padding: 18px;
  border-radius: var(--ip-radius-lg);
  background: var(--ip-color-bg-primary);
  box-shadow: var(--ip-shadow-lg);
}
.ac-title { font-size: var(--ip-text-body-size); font-weight: 600; color: var(--ip-color-text-primary); margin: 0 0 12px; }
.ac-sub { font-weight: 400; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); margin-left: 8px; }

.ac-crop-wrap {
  height: 280px;
  border-radius: var(--ip-radius-md);
  overflow: hidden;
  background: var(--ip-gray-950);
}
.ac-cropper { width: 100%; height: 100%; }

.ac-empty {
  height: 180px;
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
.ac-zoom-btn:hover { color: var(--ip-primary-600); border-color: var(--ip-primary-400); }
.ac-err { font-size: var(--ip-text-micro-size); color: var(--ip-danger-base); }
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
