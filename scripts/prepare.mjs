#!/usr/bin/env node
/* global console */
// ============================================================================
// prepare.mjs - Cross-platform dispatcher for prepare-* scripts
// ----------------------------------------------------------------------------
// Why: tauri.conf.json beforeBuildCommand invokes `pnpm prepare:pdfium`; the
// original npm script was hardcoded to powershell (Windows-only). This
// dispatcher keeps the same npm script name working on every platform:
//   - win32   -> powershell -ExecutionPolicy Bypass -File scripts/prepare-pdfium.ps1
//   - darwin/linux -> bash scripts/prepare-pdfium.sh
//
// Usage: node scripts/prepare.mjs <pdfium> [extra args passed through]
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
if (arg !== 'pdfium') {
  console.error('[prepare] usage: node scripts/prepare.mjs <pdfium>')
  process.exit(2)
}
const passThrough = process.argv.slice(3)

const isWindows = process.platform === 'win32'
const ps1 = path.join(REPO_ROOT, 'scripts', 'prepare-pdfium.ps1')
const sh = path.join(REPO_ROOT, 'scripts', 'prepare-pdfium.sh')

const command = isWindows ? 'powershell' : 'bash'
const args = isWindows
  ? ['-ExecutionPolicy', 'Bypass', '-File', ps1, ...passThrough]
  : [sh, ...passThrough]

console.log(`[prepare] ${arg} -> ${command} ${args.join(' ')}`)

const child = spawn(command, args, { stdio: 'inherit', shell: isWindows })
child.on('exit', (code) => process.exit(code ?? 1))
