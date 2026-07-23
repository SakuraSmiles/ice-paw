<script setup lang="ts">
// PawBrandMark — 猫爪品牌签名图标（Wave 2 共享组件）
//
// 职责：
//   - 渲染 IcePaw 品牌猫爪图标（inline SVG，不引用外部文件）
//   - 支持 filled / outline 两种模式
//   - 支持 hover paw-sway 动画
//   - 支持 24 / 32 / 48 三档尺寸
//
// 用法：
//   <PawBrandMark :size="24" />          渐变填充 + hover 动画
//   <PawBrandMark :size="48" :filled="false" />  描边模式 + 无动画

import { useId } from "vue";

withDefaults(
  defineProps<{
    /** 渲染尺寸（像素） */
    size?: number;
    /** 是否带 hover 动画（默认 true） */
    animated?: boolean;
    /** true = 渐变填充；false = outline-only */
    filled?: boolean;
  }>(),
  {
    size: 24,
    animated: true,
    filled: true,
  },
);

const uid = useId();
</script>

<template>
  <span
    class="paw-brand-mark"
    :class="{ 'paw-brand-mark--animated': animated !== false }"
    :style="{ width: `${size}px`, height: `${size}px` }"
    role="img"
    aria-label="IcePaw"
  >
    <svg viewBox="0 0 32 32" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <defs>
        <linearGradient :id="`paw-grad-${uid}`" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stop-color="var(--ip-primary-400)" />
          <stop offset="100%" stop-color="var(--ip-primary-600)" />
        </linearGradient>
      </defs>
      <rect
        width="32"
        height="32"
        rx="7"
        :fill="filled === false ? 'transparent' : `url(#paw-grad-${uid})`"
        :stroke="filled === false ? 'var(--ip-primary-500)' : 'none'"
        stroke-width="1.5"
      />
      <g
        transform="translate(4, 4) scale(1.45)"
        :fill="filled === false ? 'var(--ip-primary-500)' : '#FFFFFF'"
      >
        <ellipse cx="6" cy="6" rx="1.7" ry="2.3" transform="rotate(-25 6 6)" />
        <ellipse cx="11" cy="2.8" rx="1.7" ry="2.3" />
        <ellipse cx="16" cy="6" rx="1.7" ry="2.3" transform="rotate(25 16 6)" />
        <path d="M 4.5 12.5 Q 4.5 9.8, 7.8 9.3 L 14.2 9.3 Q 17.5 9.8, 17.5 12.5 Q 17.5 17.2, 11 18.2 Q 4.5 17.2, 4.5 12.5 Z" />
      </g>
    </svg>
  </span>
</template>

<style scoped>
.paw-brand-mark {
  display: inline-flex;
  flex-shrink: 0;
  cursor: default;
}
.paw-brand-mark svg {
  width: 100%;
  height: 100%;
}
.paw-brand-mark--animated {
  transition: transform var(--ip-duration-base) var(--ip-ease-out);
}
.paw-brand-mark--animated:hover {
  animation: paw-sway 1.2s var(--ip-ease-out) infinite;
}
@media (prefers-reduced-motion: reduce) {
  .paw-brand-mark--animated:hover {
    animation: none;
  }
}
</style>