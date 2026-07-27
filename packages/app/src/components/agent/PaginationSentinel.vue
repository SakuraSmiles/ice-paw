<script setup lang="ts">
// 无限滚动哨兵组件（REQ-AGENT-023）
//
// 设计：
//   - 占位元素插在列表底部
//   - 进入视口（IntersectionObserver）触发 @loadMore
//   - 父组件决定是否还有更多（hasMore）；本组件不做限制，仅负责触发
//   - 加载中显示 spinner；无更多数据时显示「已加载全部」
//   - 支持 rootMargin：提前 N px 触发，便于在 sentinel 真正进入视口前就开始加载
//   - 卸载时正确断开 observer

import { onBeforeUnmount, onMounted, ref } from "vue";
import { Loader2, Check } from "lucide-vue-next";

const props = withDefaults(
  defineProps<{
    /** 是否还有更多（true=触发加载；false=显示「已加载全部」） */
    hasMore?: boolean;
    /** 是否正在加载（true=显示 spinner） */
    loading?: boolean;
    /** 触发 IntersectionObserver 阈值（0~1，默认 0 = sentinel 露头即触发） */
    threshold?: number;
    /** 提前触发距离（px）；与 rootMargin 配合 */
    rootMargin?: string;
    /** 已加载条数（用于显示 X/Y） */
    loaded?: number;
    /** 总条数（用于显示 X/Y） */
    total?: number;
  }>(),
  {
    hasMore: true,
    loading: false,
    threshold: 0,
    rootMargin: "200px 0px",
    loaded: 0,
    total: 0,
  },
);

const emit = defineEmits<{
  /** 哨兵进入视口（父组件应判断 hasMore 后再 loadMore） */
  (e: "loadMore"): void;
  /** IntersectionObserver 状态变化（调试用） */
  (e: "visibility", visible: boolean): void;
}>();

const sentinelRef = ref<HTMLElement | null>(null);
let observer: IntersectionObserver | null = null;

function startObserver(): void {
  // SSR / 旧浏览器兜底
  if (typeof window === "undefined") return;
  if (typeof IntersectionObserver === "undefined") return;
  if (!sentinelRef.value) return;

  observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        const isVisible = entry.isIntersecting;
        emit("visibility", isVisible);
        // 仅在「视口可见 + 还有更多 + 未在加载」时才触发
        if (isVisible && props.hasMore && !props.loading) {
          emit("loadMore");
        }
      }
    },
    {
      threshold: props.threshold,
      // 默认 200px 提前量：sentinel 距视口底部还有 200px 就触发，
      // 用户滚动到位时下一页数据已经回到
      rootMargin: props.rootMargin,
    },
  );
  observer.observe(sentinelRef.value);
}

function stopObserver(): void {
  if (observer) {
    observer.disconnect();
    observer = null;
  }
}

onMounted(() => {
  // 若初始时 sentinel 已在视口（例如列表很短），则立刻触发一次加载
  // 否则在 startObserver 里也会被 observer 回调捕获
  startObserver();
});

onBeforeUnmount(() => {
  stopObserver();
});

// 暴露给父组件：哨兵已经暴露在视口时手动重置 observer
// （典型场景：父组件异步追加 items 后，列表高度变化，sentinel 可能还在视口）
defineExpose({
  reobserve(): void {
    stopObserver();
    startObserver();
  },
});
</script>

<template>
  <div ref="sentinelRef" class="agent-pagination-sentinel" aria-hidden="false">
    <!-- 加载中：spinner + 文字 -->
    <div v-if="loading" class="sentinel-state sentinel-loading" role="status">
      <Loader2 :size="16" class="sentinel-spinner" aria-hidden="true" />
      <span class="sentinel-text">加载更多…</span>
    </div>

    <!-- 还有更多 + 未在加载：占位 + 不显式提示（哨兵接近底部时才需要） -->
    <div v-else-if="hasMore" class="sentinel-state sentinel-idle">
      <span class="sentinel-text sentinel-text-faint">向下滚动加载更多</span>
    </div>

    <!-- 已无更多：终止提示 -->
    <div v-else class="sentinel-state sentinel-end" role="status">
      <Check :size="14" aria-hidden="true" />
      <span class="sentinel-text">已加载全部</span>
      <span v-if="total > 0" class="sentinel-count">({{ loaded }}/{{ total }})</span>
    </div>
  </div>
</template>

<style scoped>
.agent-pagination-sentinel {
  /* 占一个稳定行高，避免空 sentinel 触发 observer 抖动 */
  min-height: 44px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-3) 0;
}

.sentinel-state {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  line-height: 1.4;
  color: var(--ip-color-text-tertiary);
}

.sentinel-text-faint {
  opacity: 0.6;
}

.sentinel-count {
  color: var(--ip-color-text-tertiary);
  opacity: 0.7;
  font-variant-numeric: tabular-nums;
  margin-left: 2px;
}

/* 复用 UI 库的 ip-spin 关键帧（已全局注册） */
.sentinel-spinner {
  animation: ip-spin var(--ip-duration-spinner, 720ms) linear infinite;
  color: var(--ip-color-text-tertiary);
}
</style>