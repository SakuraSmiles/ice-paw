<script setup lang="ts">
// Keychain 加密存储测试页
// 演示：保存 API Key -> 读取 -> 删除 -> 列表
// 注意：仅在 Tauri 原生窗口中可用；纯浏览器下调用会失败，UI 会友好提示。

import { ref, onMounted } from "vue";
import keychain from "../utils/keychain";
import type { KeychainEntry } from "../utils/keychain";

// 环境检测
const isTauri = ref<boolean>(false);

// 表单输入
const inputProvider = ref<string>("openai");
const inputApiKey = ref<string>("");
const inputBaseUrl = ref<string>("");

// 已保存列表
interface DisplayRow {
  provider: string;
  maskedKey: string;
  baseUrl: string;
  fullEntry: KeychainEntry;
}
const savedRows = ref<DisplayRow[]>([]);

// 操作日志
interface OpRecord {
  message: string;
  ok: boolean;
  duration: number;
  detail?: string;
}
const ops = ref<OpRecord[]>([]);

/** 工具：执行一个操作并记录结果 */
async function runOp(label: string, fn: () => Promise<string>): Promise<void> {
  const start = performance.now();
  try {
    const detail = await fn();
    const duration = performance.now() - start;
    ops.value.unshift({
      message: `${label} 成功 [OK]`,
      ok: true,
      duration,
      detail,
    });
  } catch (err) {
    const duration = performance.now() - start;
    const msg = err instanceof Error ? err.message : String(err);
    ops.value.unshift({
      message: `${label} 失败 [FAIL]`,
      ok: false,
      duration,
      detail: msg,
    });
  }
}

/** 对 API Key 进行脱敏：保留前 6 位，其余替换为 *** */
function maskApiKey(apiKey: string): string {
  if (apiKey.length <= 6) {
    return apiKey + "***";
  }
  return apiKey.slice(0, 6) + "***";
}

/** 刷新已保存列表 */
async function refreshList(): Promise<void> {
  const providers = await keychain.listProviders();
  const rows: DisplayRow[] = [];
  for (const provider of providers) {
    const entry = await keychain.getKey(provider);
    if (entry) {
      rows.push({
        provider: entry.provider,
        maskedKey: maskApiKey(entry.apiKey),
        baseUrl: entry.baseUrl ?? "默认",
        fullEntry: entry,
      });
    }
  }
  // 按 provider 排序，保持列表稳定
  rows.sort((a, b) => a.provider.localeCompare(b.provider));
  savedRows.value = rows;
}

/** 保存 API Key */
async function handleSave(): Promise<void> {
  const provider = inputProvider.value;
  const apiKey = inputApiKey.value;
  const baseUrl = inputBaseUrl.value;
  await runOp("保存 key", async () => {
    await keychain.saveKey({
      provider,
      apiKey,
      baseUrl: baseUrl.trim().length > 0 ? baseUrl.trim() : undefined,
    });
    await refreshList();
    return `provider=${provider} 已保存`;
  });
}

/** 读取指定 provider 的 API Key */
async function handleRead(provider: string): Promise<void> {
  await runOp("读取 key", async () => {
    const entry = await keychain.getKey(provider);
    if (!entry) {
      return `provider=${provider} 未找到记录`;
    }
    return `provider=${provider}, apiKey=${maskApiKey(entry.apiKey)}, baseUrl=${entry.baseUrl ?? "默认"}, updatedAt=${entry.updatedAt}`;
  });
}

/** 删除指定 provider 的 API Key */
async function handleDelete(provider: string): Promise<void> {
  await runOp("删除 key", async () => {
    const removed = await keychain.deleteKey(provider);
    if (removed) {
      await refreshList();
      return `provider=${provider} 已删除`;
    }
    return `provider=${provider} 不存在，未删除`;
  });
}

/** 检查指定 provider 是否已配置 */
async function handleCheck(provider: string): Promise<void> {
  await runOp("检查 key", async () => {
    const exists = await keychain.hasKey(provider);
    return `provider=${provider} ${exists ? "已配置" : "未配置"}`;
  });
}

onMounted(() => {
  isTauri.value = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
});
</script>

<template>
  <main class="test-keychain">
    <h1 class="title">[Keychain] API Key 加密存储测试</h1>
    <p class="subtitle">
      演示 tauri-plugin-store 的加密存储能力。注意：此页面仅在 Tauri 原生窗口中可用。
    </p>

    <!-- Tauri 环境检测提示 -->
    <div v-if="!isTauri" class="banner banner-warn">
      [!] 当前不在 Tauri 原生窗口中（检测不到 <code>__TAURI_INTERNALS__</code>），
      下方按钮点击后会报错。请通过 <code>pnpm tauri dev</code> 在原生窗口中运行。
    </div>
    <div v-else class="banner banner-ok">
      [OK] 已检测到 Tauri 环境，Keychain 加密存储可用。
    </div>

    <!-- 操作区：保存表单 -->
    <section class="actions">
      <h2 class="section-title">操作区</h2>
      <div class="form-row">
        <label class="form-label" for="provider-input">Provider:</label>
        <input
          id="provider-input"
          v-model="inputProvider"
          class="form-input"
          type="text"
          placeholder="openai / glm / deepseek ..."
          :disabled="!isTauri"
        />
      </div>
      <div class="form-row">
        <label class="form-label" for="apikey-input">API Key:</label>
        <input
          id="apikey-input"
          v-model="inputApiKey"
          class="form-input"
          type="password"
          placeholder="sk-xxxx..."
          :disabled="!isTauri"
        />
      </div>
      <div class="form-row">
        <label class="form-label" for="baseurl-input">Base URL:</label>
        <input
          id="baseurl-input"
          v-model="inputBaseUrl"
          class="form-input"
          type="text"
          placeholder="（可选）自定义 API 地址"
          :disabled="!isTauri"
        />
      </div>
      <button
        class="btn btn-primary"
        :disabled="!isTauri"
        @click="handleSave"
      >
        保存 Key
      </button>
    </section>

    <!-- 已保存列表 -->
    <section class="result-section">
      <h2 class="section-title">已保存列表</h2>
      <table v-if="savedRows.length > 0">
        <thead>
          <tr>
            <th>Provider</th>
            <th>API Key</th>
            <th>Base URL</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in savedRows" :key="row.provider">
            <td><code>{{ row.provider }}</code></td>
            <td class="mono">{{ row.maskedKey }}</td>
            <td>{{ row.baseUrl }}</td>
            <td class="action-cells">
              <button
                class="btn btn-sm"
                :disabled="!isTauri"
                @click="handleRead(row.provider)"
              >
                查看
              </button>
              <button
                class="btn btn-sm btn-check"
                :disabled="!isTauri"
                @click="handleCheck(row.provider)"
              >
                检查
              </button>
              <button
                class="btn btn-sm btn-danger"
                :disabled="!isTauri"
                @click="handleDelete(row.provider)"
              >
                删除
              </button>
            </td>
          </tr>
        </tbody>
      </table>
      <p v-else class="empty-hint">暂无已保存的 Key，点击「保存 Key」试试</p>
    </section>

    <!-- 操作日志 -->
    <section class="log">
      <h2 class="section-title">操作日志</h2>
      <ul v-if="ops.length > 0">
        <li v-for="(op, idx) in ops" :key="idx" :class="op.ok ? 'op-ok' : 'op-fail'">
          <span class="op-msg">{{ op.message }}</span>
          <span class="op-duration">{{ op.duration.toFixed(1) }} ms</span>
          <pre v-if="op.detail" class="op-detail">{{ op.detail }}</pre>
        </li>
      </ul>
      <p v-else class="empty-hint">暂无操作</p>
    </section>
  </main>
</template>

<style scoped>
.test-keychain {
  margin: 0;
  padding: 2rem 1.5rem;
  max-width: 1080px;
  margin: 0 auto;
}

.title {
  font-size: 2rem;
  margin: 0 0 0.5rem;
}

.subtitle {
  color: #6b7280;
  margin: 0 0 1.5rem;
}

.banner {
  padding: 0.75rem 1rem;
  border-radius: 8px;
  margin-bottom: 1.5rem;
  font-size: 0.95rem;
  line-height: 1.5;
}

.banner code {
  background: rgba(0, 0, 0, 0.06);
  padding: 0.05em 0.35em;
  border-radius: 4px;
  font-size: 0.9em;
}

.banner-warn {
  background: #fff7e6;
  border: 1px solid #ffd591;
  color: #874d00;
}

.banner-ok {
  background: #f6ffed;
  border: 1px solid #b7eb8f;
  color: #237804;
}

.section-title {
  font-size: 1.15rem;
  margin: 0 0 0.75rem;
}

.actions {
  margin-bottom: 2rem;
}

.form-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 0.75rem;
}

.form-label {
  font-weight: 500;
  min-width: 80px;
  font-size: 0.95rem;
}

.form-input {
  flex: 1;
  padding: 0.5rem 0.75rem;
  border-radius: 8px;
  border: 1px solid #d1d5db;
  font-size: 0.95rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  background: #ffffff;
  color: #111827;
  transition: border-color 0.15s ease;
}

.form-input:focus {
  outline: none;
  border-color: #4f8cff;
  box-shadow: 0 0 0 3px rgba(79, 140, 255, 0.15);
}

.form-input:disabled {
  background: #f3f4f6;
  color: #9ca3af;
  cursor: not-allowed;
}

.result-section {
  margin-bottom: 2rem;
  overflow-x: auto;
}

table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.9rem;
}

th,
td {
  border: 1px solid #e5e7eb;
  padding: 0.5rem 0.75rem;
  text-align: left;
}

th {
  background: #f9fafb;
  font-weight: 600;
}

td code {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 0.85em;
}

.mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}

.action-cells {
  white-space: nowrap;
}

.empty-hint {
  color: #9ca3af;
  font-style: italic;
  margin: 0.5rem 0 1.5rem;
}

.btn {
  border-radius: 8px;
  border: 1px solid #d1d5db;
  background: #ffffff;
  padding: 0.55em 1.1em;
  font-size: 0.95em;
  font-weight: 500;
  color: #111827;
  cursor: pointer;
  transition: all 0.15s ease;
}

.btn:hover {
  border-color: #4f8cff;
  color: #4f8cff;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-primary {
  background: #4f8cff;
  border-color: #4f8cff;
  color: #ffffff;
}

.btn-primary:hover {
  background: #3a78eb;
  border-color: #3a78eb;
  color: #ffffff;
}

.btn-sm {
  padding: 0.3em 0.7em;
  font-size: 0.85em;
}

.btn-check {
  border-color: #6366f1;
  color: #6366f1;
}

.btn-check:hover {
  background: #6366f1;
  color: #ffffff;
}

.btn-danger {
  border-color: #ef4444;
  color: #ef4444;
}

.btn-danger:hover {
  background: #ef4444;
  color: #ffffff;
}

.log ul {
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.log li {
  padding: 0.6rem 0.85rem;
  border-radius: 8px;
  border-left: 3px solid #d1d5db;
  background: #f9fafb;
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  align-items: baseline;
}

.op-ok {
  border-left-color: #10b981;
  background: #ecfdf5;
}

.op-fail {
  border-left-color: #ef4444;
  background: #fef2f2;
}

.op-msg {
  font-weight: 500;
}

.op-duration {
  font-size: 0.8rem;
  color: #6b7280;
  margin-left: auto;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}

.op-detail {
  width: 100%;
  margin: 0.25rem 0 0;
  padding: 0.4rem 0.6rem;
  background: rgba(0, 0, 0, 0.04);
  border-radius: 4px;
  font-size: 0.8rem;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}

@media (prefers-color-scheme: dark) {
  .subtitle,
  .empty-hint,
  .op-duration {
    color: #9ca3af;
  }
  .btn {
    background: #1f2937;
    border-color: #374151;
    color: #f3f4f6;
  }
  .btn:hover {
    border-color: #4f8cff;
    color: #4f8cff;
  }
  .btn-danger:hover {
    background: #ef4444;
    color: #ffffff;
  }
  .form-input {
    background: #1f2937;
    border-color: #374151;
    color: #f3f4f6;
  }
  .form-input:focus {
    border-color: #4f8cff;
  }
  .form-input:disabled {
    background: #111827;
    color: #6b7280;
  }
  th {
    background: #1f2937;
  }
  th,
  td {
    border-color: #374151;
  }
  .log li {
    background: #1f2937;
  }
  .op-detail {
    background: rgba(255, 255, 255, 0.04);
  }
}
</style>
