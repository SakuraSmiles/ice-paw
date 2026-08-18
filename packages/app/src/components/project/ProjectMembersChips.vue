<script setup lang="ts">
// ProjectMembersChips.vue — 项目成员 chips（已选 × 移除 / 候选 + 添加）共享组件。
// 双入口复用：ProjectList 展开区 + 项目详情页设置 tab。agent 名单解析内部
// 走 agent store；增删语义「立即持久化」上交父级（emit add/remove → 父级调
// bridge 后 reload），本组件不持有中间态——两入口行为天然一致。
import { computed } from "vue";
import { useAgentStore } from "../../stores/agent";

const props = defineProps<{ memberIds: string[] }>();
const emit = defineEmits<{
  add: [agentId: string];
  remove: [agentId: string];
}>();

const agent = useAgentStore();
const memberSet = computed(() => new Set(props.memberIds));
/** 未入项目的 agent = 候选（+ 前缀） */
const candidates = computed(() => agent.list.filter((a) => !memberSet.value.has(a.id)));
</script>

<template>
  <div class="field">
    <label class="field-label">成员</label>
    <div v-if="memberIds.length === 0 && candidates.length === 0" class="members-empty">暂无可用智能体</div>
    <div v-else class="member-chips">
      <button
        v-for="m in memberIds"
        :key="'m-' + m"
        type="button"
        class="member-chip selected"
        :title="`移除 ${agent.getById(m)?.name ?? ''}`"
        @click="emit('remove', m)"
      >× {{ agent.getById(m)?.name ?? '未知' }}</button>
      <button
        v-for="a in candidates"
        :key="'a-' + a.id"
        type="button"
        class="member-chip"
        :title="`添加 ${a.name}`"
        @click="emit('add', a.id)"
      >+ {{ a.name }}</button>
    </div>
  </div>
</template>

<style scoped>
/* 样式自持（从 ProjectList 编辑区原样搬入），不依赖父级 scoped CSS */
.field { display: flex; flex-direction: column; gap: 6px; }
.field-label {
  font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  display: flex; align-items: center; gap: 6px;
}

.members-empty { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.member-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.member-chip {
  height: 28px; padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-full); cursor: pointer;
  font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.member-chip:hover { border-color: var(--ip-primary-300); }
.member-chip.selected {
  color: var(--ip-color-primary-tint-text); background-color: var(--ip-color-primary-tint-bg);
  border-color: var(--ip-primary-400);
}
</style>
