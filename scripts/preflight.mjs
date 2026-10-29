// 发版门禁脚本（批次 Z / A1 交付物，2026-10-29）
//
// 背景：0.9.10/0.9.11 连续两轮「本地全绿 → CI 红」——
//   ① e48da80 本地批验漏跑 clippy，首上 CI 触发新 lint；
//   ② 0.9.11 修复时 `pnpm typecheck | tail` 管道吃掉退出码，TS6133 漏过推出。
// 根治：把 CI 的完整验证面钉成一条本地命令，全 EXIT=0 才放行。
// 步骤与 .github/workflows/ci.yml 一一对齐（Rust clippy×2 / cargo 测试 / vitest /
// typecheck / lint / build），任何一步非零立即 fail-fast 并指明红在哪步。
//
// 用法：node scripts/preflight.mjs        （或 pnpm run preflight）
// 环境要求：cargo 步骤在 packages/app/src-tauri 下运行以吃到机器级
// .cargo/config.toml 的 SODIUM_LIB_DIR（Windows 预编译 sodium）；macOS 同理。

import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import process from 'node:process';

/* global console */

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const tauriDir = path.join(repoRoot, 'packages', 'app', 'src-tauri');

// 集成测试 target 自动发现（tests/*.rs 文件名即 target 名）——
// 新增集成测试无需改本脚本（CI yml 的手抄清单是同族漂移面，勿在此复制）。
function integrationTargets() {
  const dir = path.join(tauriDir, 'tests');
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => f.endsWith('.rs'))
    .map((f) => f.replace(/\.rs$/, ''))
    .sort();
}

// Windows 预检：sodium 路径吃不到时 cargo 会以链接错误红掉，提前给出指路，
// 避免误读为「门禁脚本坏了」。（三段式：发生了什么 / 为什么 / 怎么办）
if (
  process.platform === 'win32' &&
  !existsSync(path.join(tauriDir, '.cargo', 'config.toml')) &&
  !process.env.SODIUM_LIB_DIR
) {
  console.warn(
    '[preflight] 未检测到 packages/app/src-tauri/.cargo/config.toml，也未设置 SODIUM_LIB_DIR。\n' +
      'Rust 步骤可能因 sodium 链接失败而红：复制 .cargo/config.toml.example 为 config.toml 并填入\n' +
      '本机 sodium-prebuilt 路径（gitignored 机器级文件，换机后需重建）。',
  );
}

const steps = [
  {
    name: 'cargo clippy --lib',
    cwd: tauriDir,
    cmd: ['cargo', 'clippy', '--lib', '--', '-D', 'warnings'],
  },
  {
    name: 'cargo clippy --tests',
    cwd: tauriDir,
    cmd: ['cargo', 'clippy', '--tests', '--', '-D', 'warnings'],
  },
  {
    name: 'cargo test --lib',
    cwd: tauriDir,
    cmd: ['cargo', 'test', '--lib'],
  },
  ...integrationTargets().map((t) => ({
    name: `cargo test --test ${t}`,
    cwd: tauriDir,
    cmd: ['cargo', 'test', '--test', t],
  })),
  {
    name: 'vitest（全量）',
    cwd: repoRoot,
    cmd: ['pnpm', 'test'],
  },
  {
    name: 'typecheck',
    cwd: repoRoot,
    cmd: ['pnpm', 'typecheck'],
  },
  {
    name: 'lint',
    cwd: repoRoot,
    cmd: ['pnpm', 'run', 'lint'],
  },
  {
    name: 'build @ice-paw/ui',
    cwd: repoRoot,
    cmd: ['pnpm', '--filter', '@ice-paw/ui', 'build'],
  },
  {
    name: 'build（前端产物）',
    cwd: repoRoot,
    cmd: ['pnpm', 'run', 'build'],
  },
];

let failed = null;
const t0 = Date.now();
console.log(`[preflight] 共 ${steps.length} 步（对齐 CI 面），开始逐门禁执行。\n`);

for (const step of steps) {
  const s0 = Date.now();
  console.log(`\n===== ${step.name} =====`);
  // 关键纪律（实案②）：不走 shell 管道、退出码由 spawnSync 直接捕获——
  // 任何 `xxx | tail` 形态都会吃掉非零码，本脚本禁止。
  const r = spawnSync(step.cmd[0], step.cmd.slice(1), {
    cwd: step.cwd,
    stdio: 'inherit',
    shell: true,
  });
  const secs = ((Date.now() - s0) / 1000).toFixed(1);
  if (r.status !== 0) {
    const code = r.status === null ? `信号 ${r.signal}` : `EXIT=${r.status}`;
    console.error(`\n[preflight] ✗ 第「${step.name}」步失败（${code}，耗时 ${secs}s）——门禁未过，后续步骤已跳过。`);
    failed = step.name;
    break;
  }
  console.log(`[preflight] ✓ ${step.name}（${secs}s）`);
}

const total = ((Date.now() - t0) / 1000).toFixed(1);
if (failed) {
  console.error(`\n[preflight] 结果：未过（红在「${failed}」，总耗时 ${total}s）。修复后重跑本脚本。`);
  process.exit(1);
}
console.log(`\n[preflight] 结果：全绿（${steps.length} 步，总耗时 ${total}s）——可以发版/推送。`);
