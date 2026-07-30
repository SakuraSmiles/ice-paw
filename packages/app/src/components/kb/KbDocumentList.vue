<script setup lang="ts">
// KbDocumentList.vue — 知识库文档列表（统一组件，global / agent 复用）
// 传入 scope（+ agent 的 ownerId），组件自行解析对应 KB 并展示其文档。
import { ref, computed, onMounted } from "vue";
import { bridge } from "../../api/bridge";
import type { Kb, KbDocument, IndexStats } from "../../types";

const props = defineProps<{
  scope: "global" | "agent";
  /** scope='agent' 时传 agent.id；global 不传 */
  ownerId?: string;
}>();

const kb = ref<Kb | null>(null);
const documents = ref<KbDocument[]>([]);
const loading = ref(true);
const reindexing = ref(false);
const reindexResult = ref<string | null>(null);

const matched = (k: Kb) =>
  props.scope === "global" ? k.owner_id === null : k.owner_id === props.ownerId;

async function load() {
  loading.value = true;
  try {
    const all = await bridge.kb.list();
    kb.value = all.find((k) => k.scope === props.scope && matched(k)) ?? null;
    documents.value = kb.value
      ? await bridge.kb.listDocuments(kb.value.id)
      : [];
  } catch (e) {
    console.error("加载知识库失败:", e);
  } finally {
    loading.value = false;
  }
}

onMounted(load);

async function reindex() {
  if (!kb.value || reindexing.value) return;
  reindexing.value = true;
  reindexResult.value = null;
  try {
    const stats: IndexStats = await bridge.kb.reindex(kb.value.id);
    reindexResult.value = `已索引 ${stats.indexed} · 跳过 ${stats.skipped} · 删除 ${stats.deleted}`;
    documents.value = await bridge.kb.listDocuments(kb.value.id);
    setTimeout(() => { reindexResult.value = null; }, 3000);
  } catch (e) {
    console.error("重建索引失败:", e);
    reindexResult.value = "重建索引失败";
  } finally {
    reindexing.value = false;
  }
}

function parseTags(tags: string): string[] {
  try {
    const arr = JSON.parse(tags);
    return Array.isArray(arr)
      ? arr.filter((t): t is string => typeof t === "string")
      : [];
  } catch {
    return [];
  }
}

/** 标题为空时用文件名兜底 */
function docTitle(doc: KbDocument): string {
  if (doc.title.trim()) return doc.title;
  const name = doc.file_path.split("/").pop() ?? doc.file_path;
  return name.replace(/\.md$/i, "");
}

/** indexed_at 形如 '2026-07-30 14:23:21'，截取可读片段 */
function indexedTime(iso: string): string {
  return iso.replace("T", " ").slice(0, 16);
}

const directoryShort = computed(() => {
  const d = kb.value?.directory ?? "";
  // 路径过长时只保留末两段，title 显示完整
  const parts = d.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.length > 3 ? ".../" + parts.slice(-2).join("/") : d;
});
</script>

<template>
  <div class="kb-doc-list">
    <!-- 顶部：目录 + 文档数 + 重建索引 -->
    <div v-if="kb" class="kb-bar">
      <div class="kb-meta" :title="kb.directory">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
        </svg>
        <span class="kb-dir">{{ directoryShort }}</span>
        <span class="kb-sep">·</span>
        <span>{{ documents.length }} 篇</span>
      </div>
      <button
        class="kb-reindex-btn"
        :disabled="reindexing"
        :title="'重新扫描目录建索引'"
        @click="reindex"
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" :class="{ spinning: reindexing }">
          <polyline points="23 4 23 10 17 10" /><polyline points="1 20 1 14 7 14" />
          <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
        </svg>
        {{ reindexing ? "索引中" : "重建索引" }}
      </button>
    </div>
    <div v-if="reindexResult" class="kb-reindex-tip">{{ reindexResult }}</div>

    <!-- 加载中 -->
    <div v-if="loading" class="kb-hint">加载中…</div>

    <!-- KB 未初始化（ensure 未跑/失败） -->
    <div v-else-if="!kb" class="kb-hint">
      知识库尚未初始化，重启应用后将自动创建。
    </div>

    <!-- 文档列表 -->
    <div v-else-if="documents.length" class="kb-docs">
      <div v-for="doc in documents" :key="doc.id" class="kb-doc-card">
        <div class="doc-title-row">
          <span class="doc-title">{{ docTitle(doc) }}</span>
          <span v-for="t in parseTags(doc.tags)" :key="t" class="doc-tag">{{ t }}</span>
        </div>
        <div class="doc-path">{{ doc.file_path }}</div>
        <div v-if="doc.summary" class="doc-summary">{{ doc.summary }}</div>
        <div class="doc-indexed">索引于 {{ indexedTime(doc.indexed_at) }}</div>
      </div>
    </div>

    <!-- 空状态 -->
    <div v-else class="kb-empty">
      <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" />
      </svg>
      <div class="kb-empty-title">知识库还是空的</div>
      <div class="kb-empty-desc">
        在对话里告诉 agent 资料内容，它会自动整理入库；<br />或直接把 .md 文件放进
        <code>{{ directoryShort }}</code>
      </div>
    </div>
  </div>
</template>

<style scoped>
.kb-doc-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* ===== 顶部条 ===== */
.kb-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.kb-meta {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  min-width: 0;
}
.kb-dir {
  font-family: var(--ip-font-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.kb-sep {
  opacity: 0.5;
  margin: 0 2px;
}
.kb-reindex-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  height: 26px;
  padding: 0 10px;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.kb-reindex-btn:hover:not(:disabled) {
  color: var(--ip-primary-600);
  border-color: var(--ip-primary-300);
}
.kb-reindex-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.kb-reindex-btn svg {
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.kb-reindex-btn svg.spinning {
  animation: kb-spin 0.8s linear infinite;
}
@keyframes kb-spin {
  to { transform: rotate(360deg); }
}

.kb-reindex-tip {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-success-text);
}

.kb-hint {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  padding: 8px 0;
}

/* ===== 文档卡片 ===== */
.kb-docs {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.kb-doc-card {
  padding: 10px 12px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.kb-doc-card:hover {
  border-color: var(--ip-primary-300);
}
.doc-title-row {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.doc-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.doc-tag {
  font-size: 10px;
  padding: 0 6px;
  line-height: 18px;
  color: var(--ip-primary-700);
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
}
.doc-path {
  margin-top: 3px;
  font-size: var(--ip-text-caption-size);
  font-family: var(--ip-font-mono);
  color: var(--ip-color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.doc-summary {
  margin-top: 4px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.doc-indexed {
  margin-top: 4px;
  font-size: 10px;
  color: var(--ip-color-text-disabled);
}

/* ===== 空状态 ===== */
.kb-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 20px 12px;
  color: var(--ip-color-text-tertiary);
  text-align: center;
}
.kb-empty-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}
.kb-empty-desc {
  font-size: var(--ip-text-caption-size);
  line-height: 1.6;
}
.kb-empty-desc code {
  font-family: var(--ip-font-mono);
  font-size: 10px;
  padding: 1px 4px;
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-sm);
  word-break: break-all;
}
</style>
