<script setup lang="ts">
// ProjectDetailLayout.vue — 项目详情页布局（MA-2）：头部（返回 + 项目名 +
// 描述）+ tab 条（概览·任务台账 / 项目轨迹 / 设置）+ keep-alive 内容区。
// 先例 SettingsLayout，差异：路由带 :id 参数。⚠️ keep-alive 的 component 必须
// `:key="route.path"`：KeepAlive 以 vnode.key 为缓存键，若只按项目 id（三 tab
// 同 key），切 tab 时缓存命中旧 tab 实例直接嫁接——URL 变了视图冻结（真机踩坑）。
// route.path 同时含项目 id + tab，跨项目不串数据、tab 间各留缓存。
// 进入详情页不改变侧栏 scope：「看项目」与「切空间工作」是两个动作。
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useProjectStore } from "../../stores/project";
import { useAgentStore } from "../../stores/agent";
import { parseDbTime } from "../../utils/time";

const route = useRoute();
const router = useRouter();
const project = useProjectStore();
const agentStore = useAgentStore();

const projectId = computed(() => String(route.params.id ?? ""));
const current = computed(() => project.getById(projectId.value));
const loadError = ref(false);

const tabs = [
  { key: "overview", label: "概览 · 任务台账" },
  { key: "timeline", label: "项目轨迹" },
  { key: "settings", label: "设置" },
];
const activeTab = computed(() => {
  const seg = route.path.split("/").pop() || "overview";
  return tabs.some((t) => t.key === seg) ? seg : "overview";
});

function navigate(key: string) {
  router.push(`/projects/${projectId.value}/${key}`);
}

onMounted(async () => {
  // agent 名单（头部成员 meta / 台账执行者列共用；store 内 loaded 幂等）
  void agentStore.load();
  // 直链进入时 store 可能未加载（刷新/外部跳转）
  if (!current.value) {
    try {
      await project.load(true);
      if (!current.value) loadError.value = true;
    } catch {
      loadError.value = true;
    }
  }
});

/** 成员名（卡片头部 meta；agent store onMounted 拉取） */
const memberSummary = computed(() => {
  const names = (current.value?.agents ?? [])
    .map((a) => agentStore.getById(a.agent_id)?.name)
    .filter(Boolean) as string[];
  return names.length ? names.join("、") : "未分配成员";
});

const updatedLabel = computed(() => {
  const t = current.value ? parseDbTime(current.value.updated_at) : null;
  return t ? t.toLocaleString() : "";
});
</script>

<template>
  <div class="detail-page">
    <header class="detail-header">
      <button class="back-btn" title="返回项目列表" @click="router.push('/projects')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="19" y1="12" x2="5" y2="12" /><polyline points="12 19 5 12 12 5" />
        </svg>
      </button>
      <div class="head-main">
        <template v-if="current">
          <h1 class="head-name">{{ current.name }}</h1>
          <div class="head-meta">
            <span v-if="current.description" class="head-desc">{{ current.description }}</span>
            <span class="head-members">{{ memberSummary }}</span>
            <span v-if="updatedLabel" class="head-updated">更新于 {{ updatedLabel }}</span>
          </div>
        </template>
        <h1 v-else-if="loadError" class="head-name err">项目不存在或已删除</h1>
        <h1 v-else class="head-name">加载中…</h1>
      </div>
    </header>

    <nav v-if="current" class="tab-bar">
      <button
        v-for="t in tabs"
        :key="t.key"
        class="tab-item"
        :class="{ active: activeTab === t.key }"
        @click="navigate(t.key)"
      >{{ t.label }}</button>
    </nav>

    <div class="detail-body">
      <div v-if="loadError" class="load-error">
        找不到该项目。<button class="btn-link" @click="router.push('/projects')">返回项目列表</button>
      </div>
      <router-view v-else v-slot="{ Component }">
        <keep-alive>
          <component :is="Component" :key="route.path" />
        </keep-alive>
      </router-view>
    </div>
  </div>
</template>

<style scoped>
.detail-page {
  height: 100%;
  display: flex;
  flex-direction: column;
  background-color: var(--ip-color-bg-primary);
}

.detail-header {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px 32px;
  min-height: 68px;
  border-bottom: 1px solid var(--color-chat-header-border);
  background-color: var(--color-chat-header-bg);
  backdrop-filter: blur(8px);
  flex-shrink: 0;
}

.back-btn {
  display: flex; align-items: center; justify-content: center;
  width: 30px; height: 30px; flex-shrink: 0;
  border: none; border-radius: var(--ip-radius-md);
  background: none; cursor: pointer;
  color: var(--ip-color-text-secondary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.back-btn:hover { background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }

.head-main { flex: 1; min-width: 0; }
.head-name {
  margin: 0;
  font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.head-name.err { color: var(--ip-danger-text); }
.head-meta {
  display: flex; align-items: center; gap: 10px;
  margin-top: 2px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
}
.head-desc {
  min-width: 0; max-width: 40%;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.head-members { color: var(--ip-color-text-secondary); }
.head-updated { color: var(--ip-color-text-disabled); }

.tab-bar {
  display: flex; gap: 4px;
  padding: 0 32px;
  border-bottom: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}
.tab-item {
  position: relative;
  padding: 10px 12px;
  border: none; background: none; cursor: pointer;
  font-family: inherit; font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.tab-item:hover { color: var(--ip-color-text-primary); }
.tab-item.active { color: var(--ip-primary-600); font-weight: var(--ip-font-weight-medium); }
.tab-item.active::after {
  content: ""; position: absolute; left: 10px; right: 10px; bottom: -1px;
  height: 2px; border-radius: 1px; background: var(--ip-primary-500);
}

.detail-body {
  flex: 1; min-height: 0;
  padding: 20px 32px 24px;
  display: flex; flex-direction: column;
}
.load-error {
  padding: 24px; color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-body-sm-size);
}
.btn-link {
  height: 30px; padding: 0 8px; background: none; border: none; cursor: pointer;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-primary-600); font-family: inherit;
}
</style>
