<!--
  TrajectoryKindFilter — 「类型」事件类型多选下拉（轨迹筛选的状态编辑器）

  设计定位（与「仅对话」药丸同源，见 useTrajectory.ts 筛选模型注释）：
  「仅对话」= 高频预设（1 击），本组件 = 低频细筛编辑器；两者操纵同一
  hiddenKinds 状态，无第二状态源——预设态在此所见即所是（勾选即可见性）。

  范式循 MoreMenu：capture 阶段 document click 点外关闭（穿透 @click.stop 容器）、
  Transition + v-if 开合。复选行循 tbar-toggle 的隐藏原生 checkbox + CSS 兄弟
  选择器（原生 input 保持可聚焦，键盘 Tab+Space 可达）。
-->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { ChevronDown } from "@lucide/vue";
import { DEFAULT_HIDDEN, FILTER_GROUPS, type FilterKey } from "../../composables/useTrajectory";

const props = defineProps<{
  /** 隐藏的类型键集合（数组形态便于 v-model 语义与测试断言） */
  hidden: FilterKey[];
  /** 加载窗口内各类型事件计数（信息气味；缺省/为 0 不显示徽标） */
  counts?: Partial<Record<FilterKey, number>>;
}>();
const emit = defineEmits<{ "update:hidden": [FilterKey[]] }>();

const open = ref(false);
const wrapRef = ref<HTMLElement | null>(null);

/** 触发药丸亮态判据：隐藏集 ≠ 默认集（含「仅对话」态——两灯齐亮 = 筛选生效中，语义真） */
const isDefault = computed(() => {
  const a = new Set(props.hidden);
  const b = new Set(DEFAULT_HIDDEN);
  return a.size === b.size && [...a].every((k) => b.has(k));
});

function isChecked(key: FilterKey): boolean {
  return !props.hidden.includes(key);
}

function toggle(key: FilterKey, checked: boolean) {
  const s = new Set(props.hidden);
  if (checked) s.delete(key);
  else s.add(key);
  emit("update:hidden", [...s]);
}

function resetDefault() {
  emit("update:hidden", [...DEFAULT_HIDDEN]);
}

function onDocClick(e: MouseEvent) {
  if (open.value && wrapRef.value && !wrapRef.value.contains(e.target as Node)) open.value = false;
}
function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && open.value) open.value = false;
}
// capture 阶段监听：工具栏可能处在带 @click.stop 的容器内，冒泡会被拦导致收不到外部点击
onMounted(() => {
  document.addEventListener("click", onDocClick, true);
  document.addEventListener("keydown", onKeydown);
});
onUnmounted(() => {
  document.removeEventListener("click", onDocClick, true);
  document.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div ref="wrapRef" class="tkf-wrap">
    <button
      class="tkf-btn"
      :class="{ active: !isDefault, open }"
      title="选择显示的事件类型"
      @click="open = !open"
    >
      <span>类型</span>
      <ChevronDown :size="14" class="tkf-chev" aria-hidden="true" />
    </button>
    <Transition name="tkf-drop">
      <div v-if="open" class="tkf-pop" @click.stop>
        <template v-for="group in FILTER_GROUPS" :key="group.label">
          <div class="tkf-group">{{ group.label }}</div>
          <label
            v-for="it in group.items"
            :key="it.key"
            class="tkf-row"
            :title="isChecked(it.key) ? `隐藏${it.label}` : `显示${it.label}`"
          >
            <input
              type="checkbox"
              :checked="isChecked(it.key)"
              @change="toggle(it.key, ($event.target as HTMLInputElement).checked)"
            />
            <span class="tkf-box" aria-hidden="true" />
            <span class="tkf-label">{{ it.label }}</span>
            <span v-if="counts?.[it.key]" class="tkf-count">{{ counts[it.key] }}</span>
          </label>
        </template>
        <div v-if="!isDefault" class="tkf-foot">
          <button class="tkf-reset" @click="resetDefault">恢复默认</button>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.tkf-wrap {
  position: relative;
}

/* 触发药丸：与 .tbar-pill / .pt-pill 同语言（26px 圆角胶囊） */
.tkf-btn {
  height: 26px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 0 10px 0 12px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  white-space: nowrap;
  cursor: pointer;
  transition: var(--ip-transition-colors);
}
.tkf-btn:hover {
  color: var(--ip-color-text-primary);
}
.tkf-btn.active {
  color: var(--ip-primary-600);
  background: var(--ip-color-primary-soft-bg, var(--ip-primary-50));
  border-color: var(--ip-primary-200);
}
.tkf-chev {
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.tkf-btn.open .tkf-chev {
  transform: rotate(180deg);
}

.tkf-pop {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: var(--ip-z-dropdown);
  min-width: 216px;
  padding: var(--ip-spacing-2);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
}

.tkf-group {
  padding: var(--ip-spacing-2) 10px 2px;
  font-size: var(--ip-text-micro-size);
  font-weight: var(--ip-font-weight-semibold);
  letter-spacing: 0.02em;
  color: var(--ip-color-text-secondary);
}
.tkf-group:first-child {
  padding-top: 2px;
}

/* 复选行：整行可点（label 语义）；隐藏原生 checkbox 保持可聚焦（Tab+Space） */
.tkf-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: 4px 10px;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  user-select: none;
}
.tkf-row:hover {
  background: var(--ip-color-bg-tertiary);
}
.tkf-row input {
  position: absolute;
  opacity: 0;
  pointer-events: none;
}
/* 自绘复选方块（14px）：勾走 ::after 边框三角，选中态主题色填充 */
.tkf-box {
  flex-shrink: 0;
  width: 14px;
  height: 14px;
  border: 1px solid var(--ip-color-border-default);
  border-radius: 3px;
  background: var(--ip-color-bg-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: var(--ip-transition-colors);
}
.tkf-box::after {
  content: "";
  width: 3px;
  height: 7px;
  border: solid var(--ip-color-bg-elevated);
  border-width: 0 2px 2px 0;
  transform: rotate(45deg) translateY(-1px);
  opacity: 0;
}
.tkf-row input:checked + .tkf-box {
  background: var(--ip-primary-500);
  border-color: var(--ip-primary-500);
}
.tkf-row input:checked + .tkf-box::after {
  opacity: 1;
}
.tkf-row input:focus-visible + .tkf-box {
  box-shadow: var(--ip-shadow-focus);
}

.tkf-label {
  flex: 1;
  min-width: 0;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
}
.tkf-count {
  flex-shrink: 0;
  font-family: var(--ip-font-mono, monospace);
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-disabled);
  font-variant-numeric: tabular-nums;
}

.tkf-foot {
  margin-top: var(--ip-spacing-1);
  padding-top: var(--ip-spacing-2);
  border-top: 1px solid var(--ip-color-border-default);
}
.tkf-reset {
  padding: 2px 10px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  background: none;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
}
.tkf-reset:hover {
  color: var(--ip-primary-600);
  background: var(--ip-color-bg-tertiary);
}

/* 开合动画（MoreMenu 同款） */
.tkf-drop-enter-active {
  animation: tkf-in 0.12s ease-out;
}
.tkf-drop-leave-active {
  animation: tkf-in 0.1s ease-in reverse;
}
@keyframes tkf-in {
  from { opacity: 0; transform: translateY(-4px) scale(0.96); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}
</style>
