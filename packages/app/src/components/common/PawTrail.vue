<script setup lang="ts">
// PawTrail — 猫爪脚印装饰条（Wave 2 共享组件）
//
// 职责：
//   - 渲染 N 个小猫爪图标，错位旋转 + 断线装饰
//   - 常驻 paw-breathe 动画，每个 mark 错开延迟
//   - 用于 WelcomePanel 底部装饰
//
// 用法：
//   <PawTrail />                    默认 5 个 size=14
//   <PawTrail :count="3" />         移动端简化为 3 个

import PawBrandMark from "./PawBrandMark.vue";

withDefaults(
  defineProps<{
    /** 猫爪数量，默认 5；mobile 可传 3 */
    count?: number;
    /** 单个猫爪尺寸，默认 14 */
    size?: number;
  }>(),
  {
    count: 5,
    size: 14,
  },
);
</script>

<template>
  <div class="paw-trail">
    <template v-for="i in count" :key="i">
      <span
        class="paw-trail__mark"
        :style="{
          transform: `rotate(${(i % 2 === 0 ? -1 : 1) * (i * 4 - 10)}deg)`,
          animationDelay: `${(i - 1) * 120}ms`,
        }"
      >
        <PawBrandMark :size="size" :animated="false" />
      </span>
      <span v-if="i < count && i % 2 === 0" class="paw-trail__break" />
    </template>
  </div>
</template>

<style scoped>
.paw-trail {
  display: inline-flex;
  align-items: center;
  gap: 12px;
  color: var(--ip-primary-400);
  opacity: 0.7;
}
.paw-trail__mark {
  display: inline-flex;
  animation: paw-breathe 2.4s ease-in-out infinite;
}
.paw-trail__break {
  display: inline-block;
  width: 40px;
  height: 1px;
  background: linear-gradient(90deg, var(--ip-primary-300), transparent);
}
@media (prefers-reduced-motion: reduce) {
  .paw-trail__mark {
    animation: none;
  }
}
</style>