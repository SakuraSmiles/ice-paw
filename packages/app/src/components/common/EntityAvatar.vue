<!--
  EntityAvatar — agent 头像（项目头像功能 2026-08-22 移除后仅服务 agent 语境）

  渲染链（2026-08-22 拍板全语境统一）：image（用户上传图）→ 默认头像图
  （组件内置，所有展示位无图一律默认图）→ 名字哈希渐变 + 首字（遗留兜底：
  默认图也加载失败才出现，打包资产正常不可达）。
  Props: name: string（实体名，兜底首字与哈希来源）
         image?: string | null（图片 dataURL/base64；加载失败自动降级）
         accent?: string | null（主题色 hex，兜底档底色优先用）
         size?: 'xs' | 'sm' | 'md' | 'lg' | 'xl'（16/20/28/36/44px）
  Emits: 无
-->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import defaultAvatar from "../../assets/default-agent-avatar.png";

const props = withDefaults(
  defineProps<{
    name: string;
    image?: string | null;
    accent?: string | null;
    size?: "xs" | "sm" | "md" | "lg" | "xl";
  }>(),
  { image: null, accent: null, size: "md" },
);

/** 分层失败标记：用户图挂 → 默认图接棒；默认图也挂 → 首字遗留兜底。 */
const userFailed = ref(false);
const defaultFailed = ref(false);
// 换图（image 变化）重置失败态重试——img 元素带 :key 换新，失败标记不会自动清
watch(
  () => props.image,
  () => {
    userFailed.value = false;
    defaultFailed.value = false;
  },
);

/** 名字首字（CJK 安全：取首个 code point）。 */
const initial = computed(() => {
  const chs = Array.from((props.name || "").trim());
  return chs[0] ?? "?";
});

/**
 * 策展渐变色板（10 对，135° 对角；tint 安全、深浅双主题文字可读）。
 * 勿运行时算 hue——易出脏色；新色对须双主题下目检。
 */
const PALETTE: ReadonlyArray<readonly [string, string]> = [
  ["#4680C2", "#2A4F85"], // 品牌蓝
  ["#3BAF7A", "#1A5D42"], // 松绿
  ["#B8862A", "#7D5614"], // 琥珀
  ["#B83D3D", "#7D2323"], // 暗红
  ["#7C6BC4", "#4A3F8C"], // 靛紫
  ["#3D9DB3", "#1F5F70"], // 青
  ["#C46B9A", "#8C3D66"], // 绛紫
  ["#6FA1D6", "#3565A8"], // 浅蓝
  ["#8A9B4A", "#55632B"], // 橄榄
  ["#8D8D9E", "#545463"], // 石墨
];

/** 名字稳定哈希（FNV-1a 32 位）→ 色板下标。同名恒定、跨会话一致。 */
const gradient = computed(() => {
  const s = props.name || "";
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  const [from, to] = PALETTE[Math.abs(h) % PALETTE.length];
  return `linear-gradient(135deg, ${from}, ${to})`;
});

/** 兜底档底色：显式主题色（纯色）优先，否则哈希渐变。 */
const bg = computed(() => (props.accent ? props.accent : gradient.value));

/** 图片源三级链：用户图 → 默认头像图 → 无（走首字遗留兜底）。 */
const src = computed(() => (userFailed.value ? defaultAvatar : props.image || defaultAvatar));

/** 是否走图片级（默认图未失败即真——用户图挂由 src 换默认图接棒）。 */
const useImage = computed(() => !defaultFailed.value);

function onImgError() {
  if (props.image && !userFailed.value) userFailed.value = true;
  else defaultFailed.value = true;
}
</script>

<template>
  <span :class="['entity-avatar', `size-${size}`]" :style="{ background: useImage ? undefined : bg }">
    <img v-if="useImage" :key="src" :src="src" alt="" @error="onImgError" />
    <template v-else>{{ initial }}</template>
  </span>
</template>

<style scoped>
.entity-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  border-radius: var(--ip-radius-full, 9999px);
  color: var(--ip-white);
  font-weight: var(--ip-font-weight-semibold, 600);
  line-height: 1;
  overflow: hidden;
  user-select: none;
  /* GitHub 风格两件套：发丝描边（ring）+ 轻投影（elev），令牌三主题区同步 */
  box-shadow: var(--ip-avatar-ring, inset 0 0 0 1px rgba(0, 0, 0, 0.12)),
    var(--ip-avatar-elev, 0 1px 2px rgba(0, 0, 0, 0.08));
}

.entity-avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

/* 尺寸梯度：xs 侧栏小标签 / sm 菜单与列表行 / md 聊天头与选择器 / lg 表单预览 / xl ChatHeader 占两行高度 */
.size-xs { width: 16px; height: 16px; font-size: var(--ip-text-micro-size); }
.size-sm { width: 20px; height: 20px; font-size: var(--ip-text-micro-size); }
.size-md { width: 28px; height: 28px; font-size: 13px; }
.size-lg { width: 36px; height: 36px; font-size: 16px; }
.size-xl { width: 44px; height: 44px; font-size: 18px; }

/* 小尺寸（xs/sm）：仅描边不加投影——16-20px 上投影只剩噪点 */
.size-xs,
.size-sm {
  box-shadow: var(--ip-avatar-ring, inset 0 0 0 1px rgba(0, 0, 0, 0.12));
}

.entity-avatar { font-family: var(--ip-font-sans); }
</style>
