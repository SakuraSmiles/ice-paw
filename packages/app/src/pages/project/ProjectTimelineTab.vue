<script setup lang="ts">
// ProjectTimelineTab.vue — 项目详情「项目轨迹」tab：薄容器（路由参数 → 组件）。
// 跨会话合并事件流的全部逻辑在 components/project/ProjectTimeline.vue（复用
// TrajectoryTable/Inspector + scopeTurnKeys 适配 + 全局 id 游标分页）。
// 实例由 DetailLayout 的 keep-alive `:key=项目id` 重建——切项目即重挂载，组件内
// 不需感知项目切换。
import { computed } from "vue";
import { useRoute } from "vue-router";
import ProjectTimeline from "../../components/project/ProjectTimeline.vue";

const route = useRoute();
const projectId = computed(() => String(route.params.id ?? ""));
</script>

<template>
  <ProjectTimeline :project-id="projectId" />
</template>
