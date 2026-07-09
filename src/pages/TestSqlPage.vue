<script setup lang="ts">
// SQLite 数据库测试页
// 演示：连接 -> 建表 -> 插入 -> 查询 -> 更新 -> 删除
// 注意：仅在 Tauri 原生窗口中可用；纯浏览器下调用会失败，UI 会友好提示。

import { ref } from "vue";
import database from "../utils/database";
import { initSchema, type AgentRow } from "../utils/dbSchema";

// 状态：当前插入/操作的 agent id（用于后续的更新/删除）
const currentAgentId = ref<string | null>(null);

// 状态：当前查询出来的 agents 列表
const agents = ref<AgentRow[]>([]);

// 状态：每一步操作的结果（消息 + 耗时 + 成功/失败）
interface OpRecord {
  message: string;
  ok: boolean;
  duration: number;
  detail?: string;
}
const ops = ref<OpRecord[]>([]);

// 状态：当前是否正在 Tauri 原生窗口中（用于显示友好提示）
const isTauri = ref<boolean>(typeof window !== "undefined" && "__TAURI_INTERNALS__" in window);

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

/** 连接测试 + 建表 */
async function handleConnect(): Promise<void> {
  await runOp("连接 + 建表", async () => {
    await database.init("sqlite:icepaw.db");
    await initSchema();
    return "数据库连接成功 [OK]，三张表已创建（或已存在）";
  });
}

/** 插入测试：插入一条 agent 记录 */
async function handleInsert(): Promise<void> {
  await runOp("插入测试", async () => {
    const id = crypto.randomUUID();
    const name = `TestAgent-${id.slice(0, 6)}`;
    const model = "gpt-4";
    const result = await database.execute(
      "INSERT INTO agents (id, name, model, system_prompt) VALUES ($1, $2, $3, $4)",
      [id, name, model, "You are a helpful assistant."]
    );
    currentAgentId.value = id;
    return `插入成功：id=${id.slice(0, 8)}…, rowsAffected=${result.rowsAffected}`;
  });
}

/** 查询测试：列出所有 agent */
async function handleSelect(): Promise<void> {
  await runOp("查询测试", async () => {
    const rows = await database.select<AgentRow>(
      "SELECT id, name, model, system_prompt, created_at, updated_at FROM agents ORDER BY created_at DESC"
    );
    agents.value = rows;
    return `共 ${rows.length} 条记录`;
  });
}

/** 更新测试：把刚才插入的 agent.model 改成 glm-4 */
async function handleUpdate(): Promise<void> {
  if (!currentAgentId.value) {
    ops.value.unshift({
      message: "更新测试 失败 [FAIL]",
      ok: false,
      duration: 0,
      detail: "请先点击「插入测试」产生一条记录",
    });
    return;
  }
  const id = currentAgentId.value;
  await runOp("更新测试", async () => {
    const result = await database.execute(
      "UPDATE agents SET model = $1, updated_at = datetime('now') WHERE id = $2",
      ["glm-4", id]
    );
    return `更新成功：rowsAffected=${result.rowsAffected}`;
  });
}

/** 删除测试：删除刚才插入的 agent */
async function handleDelete(): Promise<void> {
  if (!currentAgentId.value) {
    ops.value.unshift({
      message: "删除测试 失败 [FAIL]",
      ok: false,
      duration: 0,
      detail: "请先点击「插入测试」产生一条记录",
    });
    return;
  }
  const id = currentAgentId.value;
  await runOp("删除测试", async () => {
    const result = await database.execute("DELETE FROM agents WHERE id = $1", [id]);
    currentAgentId.value = null;
    return `删除成功：rowsAffected=${result.rowsAffected}`;
  });
}
</script>

<template>
  <main class="test-sql">
    <h1 class="title">SQLite 数据库测试 [SQL]</h1>
    <p class="subtitle">
      演示 tauri-plugin-sql 的连接、建表、增删改查。注意：此页面仅在 Tauri 原生窗口中可用。
    </p>

    <!-- Tauri 环境检测提示 -->
    <div v-if="!isTauri" class="banner banner-warn">
      [!] 当前不在 Tauri 原生窗口中（检测不到 <code>__TAURI_INTERNALS__</code>），
      下方按钮点击后会报错。 请通过 <code>pnpm tauri dev</code> 在原生窗口中运行。
    </div>
    <div v-else class="banner banner-ok">
      [OK] 已检测到 Tauri 环境，SQL 功能可用。
    </div>

    <!-- 操作按钮区 -->
    <section class="actions">
      <button class="btn btn-primary" @click="handleConnect">1. 连接 + 建表</button>
      <button class="btn" @click="handleInsert">2. 插入测试</button>
      <button class="btn" @click="handleSelect">3. 查询测试</button>
      <button class="btn" @click="handleUpdate">4. 更新测试</button>
      <button class="btn btn-danger" @click="handleDelete">5. 删除测试</button>
    </section>

    <!-- 当前 agent id 提示 -->
    <section v-if="currentAgentId" class="current-id">
      当前操作中的 agent id：<code>{{ currentAgentId }}</code>
    </section>

    <!-- 查询结果表格 -->
    <section v-if="agents.length > 0" class="result-table">
      <h2>查询结果（共 {{ agents.length }} 条）</h2>
      <table>
        <thead>
          <tr>
            <th>id</th>
            <th>name</th>
            <th>model</th>
            <th>system_prompt</th>
            <th>created_at</th>
            <th>updated_at</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in agents" :key="row.id">
            <td><code>{{ row.id.slice(0, 8) }}…</code></td>
            <td>{{ row.name }}</td>
            <td>{{ row.model ?? "" }}</td>
            <td class="ellipsis">{{ row.system_prompt ?? "" }}</td>
            <td>{{ row.created_at ?? "" }}</td>
            <td>{{ row.updated_at ?? "" }}</td>
          </tr>
        </tbody>
      </table>
    </section>
    <section v-else class="empty-hint">还没有查询记录，点击「3. 查询测试」试试</section>

    <!-- 操作日志 -->
    <section class="log">
      <h2>操作日志</h2>
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
.test-sql {
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

.actions {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
  margin-bottom: 1.5rem;
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

.btn-danger {
  border-color: #ef4444;
  color: #ef4444;
}

.btn-danger:hover {
  background: #ef4444;
  color: #ffffff;
}

.current-id {
  background: #f3f4f6;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  font-size: 0.85rem;
  margin-bottom: 1rem;
  word-break: break-all;
}

.current-id code {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}

.result-table {
  margin-bottom: 1.5rem;
  overflow-x: auto;
}

.result-table h2 {
  font-size: 1.15rem;
  margin: 0 0 0.5rem;
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

td.ellipsis {
  max-width: 240px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.empty-hint {
  color: #9ca3af;
  font-style: italic;
  margin: 0.5rem 0 1.5rem;
}

.log h2 {
  font-size: 1.15rem;
  margin: 0 0 0.5rem;
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
  .current-id {
    background: #1f2937;
    color: #d1d5db;
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