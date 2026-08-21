#!/usr/bin/env node
/* global console */
// ============================================================================
// prepare.mjs - Cross-platform dispatcher for prepare-* scripts
// ----------------------------------------------------------------------------
// Why: tauri.conf.json beforeDevCommand/beforeBuildCommand invoke
// `pnpm prepare:mcp` / `pnpm prepare:pdfium`; the original npm scripts were
// hardcoded to powershell (Windows-only). This dispatcher keeps the same npm
// script names working on every platform:
//   - win32   -> powershell -ExecutionPolicy Bypass -File scripts/prepare-*.ps1
//   - darwin/linux -> bash scripts/prepare-*.sh
//
// Usage: node scripts/prepare.mjs <mcp|pdfium> [extra args passed through]
// Exit code mirrors the underlying script. Idempotency is owned by the
// underlying scripts (they skip when targets already exist).
// ============================================================================

import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import process from 'node:process'

// Repo root = parent of this script's directory — works regardless of cwd
// (beforeBuildCommand runs from packages/app/, not the repo root).
const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const arg = process.argv[2]
if (arg !== 'mcp' && arg !== 'pdfium') {
  console.error('[prepare] usage: node scripts/prepare.mjs <mcp|pdfium>')
  process.exit(2)
}
const passThrough = process.argv.slice(3)

const isWindows = process.platform === 'win32'
const ps1 = path.join(REPO_ROOT, 'scripts', `prepare-${arg === 'mcp' ? 'mcp-runtime' : 'pdfium'}.ps1`)
const sh = path.join(REPO_ROOT, 'scripts', `prepare-${arg === 'mcp' ? 'mcp-runtime' : 'pdfium'}.sh`)

const command = isWindows ? 'powershell' : 'bash'
const args = isWindows
  ? ['-ExecutionPolicy', 'Bypass', '-File', ps1, ...passThrough]
  : [sh, ...passThrough]

console.log(`[prepare] ${arg} -> ${command} ${args.join(' ')}`)

const child = spawn(command, args, { stdio: 'inherit', shell: isWindows })
child.on('exit', (code) => process.exit(code ?? 1))
