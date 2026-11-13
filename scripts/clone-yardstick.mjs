#!/usr/bin/env node
// Doc 46 §3's clone-rate measure, as a script (unit CT.0, ruling Q-46-7).
//
// It is run by hand and its figures are recorded in docs/design/46-cut-notes.md at each unit.
// It is not a CI gate.
//
// The method, exactly as §3 states it. For each root:
//   1. every `.rs` file under the root is split into lines;
//   2. each line is trimmed, and blank lines are dropped;
//   3. a line is cloned when it sits inside an 8-line window that also occurs, exactly, in
//      another file under the same root. Both sides of a clone count. A window repeated only
//      inside one file does not count.
//
// Usage, from the repository root:
//   node scripts/clone-yardstick.mjs                 the four roots: totals and a table a root
//   node scripts/clone-yardstick.mjs --top 12        also the 12 files with the most cloned lines
//   node scripts/clone-yardstick.mjs --stems         fold `name.rs` into `name/` in the table
//   node scripts/clone-yardstick.mjs --rows 20       rows a table (default 14; 0 = all)
//   node scripts/clone-yardstick.mjs --json          one JSON object, for a diff between units
//   node scripts/clone-yardstick.mjs --window 8      the window, in lines (§3 uses 8)
//   node scripts/clone-yardstick.mjs <root>...       other roots instead of the four
//
// The figures move when a file is added under a root, tests included: record the run before
// and after a unit.

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const ROOTS = [
  'crates/mush-market/src',
  'crates/mush-economy/src',
  'crates/mush-roll/src/commerce',
  'crates/mush-core/src/action_state/terminal_capacity',
];

function usage(message) {
  if (message) console.error(`clone-yardstick: ${message}`);
  console.error(
    'usage: node scripts/clone-yardstick.mjs [--top N] [--rows N] [--stems] [--json] [--window N] [root...]',
  );
  process.exit(2);
}

function parseArgs(argv) {
  const options = { top: 0, rows: 14, stems: false, json: false, window: 8, roots: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const number = (name) => {
      const value = Number.parseInt(argv[(i += 1)], 10);
      if (!Number.isInteger(value) || value < 0) usage(`${name} wants a whole number`);
      return value;
    };
    if (arg === '--top') options.top = number('--top');
    else if (arg === '--rows') options.rows = number('--rows');
    else if (arg === '--window') options.window = number('--window');
    else if (arg === '--stems') options.stems = true;
    else if (arg === '--json') options.json = true;
    else if (arg === '--help' || arg === '-h') usage();
    else if (arg.startsWith('--')) usage(`unknown option ${arg}`);
    else options.roots.push(arg);
  }
  if (options.window < 1) usage('--window wants at least 1');
  if (options.roots.length === 0) options.roots = ROOTS;
  return options;
}

// Every `.rs` file under `dir`, sorted, so that a run prints the same on every platform.
function rustFiles(dir, out = []) {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) rustFiles(full, out);
    else if (entry.isFile() && entry.name.endsWith('.rs')) out.push(full);
  }
  return out;
}

function measure(root, window) {
  const files = rustFiles(root);
  const lines = files.map((file) =>
    fs
      .readFileSync(file, 'utf8')
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line.length > 0),
  );
  // window text -> the one file that holds it, or -1 once a second file does.
  const owner = new Map();
  lines.forEach((file, index) => {
    for (let i = 0; i + window <= file.length; i += 1) {
      const key = file.slice(i, i + window).join('\n');
      const seen = owner.get(key);
      if (seen === undefined) owner.set(key, index);
      else if (seen !== index) owner.set(key, -1);
    }
  });
  let total = 0;
  let cloned = 0;
  const perFile = lines.map((file, index) => {
    const mark = new Uint8Array(file.length);
    for (let i = 0; i + window <= file.length; i += 1) {
      if (owner.get(file.slice(i, i + window).join('\n')) === -1) mark.fill(1, i, i + window);
    }
    const count = mark.reduce((sum, bit) => sum + bit, 0);
    total += file.length;
    cloned += count;
    return {
      file: path.relative(root, files[index]).split(path.sep).join('/'),
      cloned: count,
      lines: file.length,
    };
  });
  return { root, files: files.length, total, cloned, perFile };
}

// §3's second table: the first path component under the root; a file at the root itself
// goes under one heading, or (with --stems) under its own name with `.rs` dropped, which
// folds `renewal.rs` into `renewal/` as the kernel's layer is laid out.
function groups(result, stems) {
  const table = new Map();
  for (const entry of result.perFile) {
    const parts = entry.file.split('/');
    let key;
    if (parts.length > 1) key = stems ? parts[0] : `${parts[0]}/`;
    else key = stems ? parts[0].replace(/\.rs$/, '') : '(top-level files)';
    const row = table.get(key) ?? { group: key, cloned: 0, lines: 0 };
    row.cloned += entry.cloned;
    row.lines += entry.lines;
    table.set(key, row);
  }
  return [...table.values()].sort(
    (a, b) => b.cloned - a.cloned || (a.group < b.group ? -1 : a.group > b.group ? 1 : 0),
  );
}

const rate = (cloned, lines) => (lines === 0 ? '0.0' : ((100 * cloned) / lines).toFixed(1));
const grouped = (value) => value.toLocaleString('en-US');

function main() {
  const options = parseArgs(process.argv.slice(2));
  const results = options.roots.map((root) => {
    if (!fs.existsSync(root)) usage(`no such root ${root} (run from the repository root)`);
    return measure(root, options.window);
  });
  if (options.json) {
    const out = results.map((result) => ({
      root: result.root.split(path.sep).join('/'),
      files: result.files,
      cloned: result.cloned,
      lines: result.total,
      rate: Number(rate(result.cloned, result.total)),
      groups: groups(result, options.stems),
    }));
    console.log(JSON.stringify({ window: options.window, roots: out }, null, 2));
    return;
  }
  console.log(`clone yardstick (doc 46 §3): ${options.window}-line windows, trimmed, blanks dropped`);
  console.log('');
  for (const result of results) {
    console.log(
      `${result.root}: ${grouped(result.cloned)} cloned of ${grouped(result.total)} lines ` +
        `(${rate(result.cloned, result.total)}%), ${result.files} files`,
    );
  }
  for (const result of results) {
    console.log('');
    console.log(`== ${result.root}`);
    const table = groups(result, options.stems);
    const rows = options.rows === 0 ? table : table.slice(0, options.rows);
    for (const row of rows) {
      console.log(
        `  ${String(row.cloned).padStart(6)} / ${String(row.lines).padStart(6)}  ` +
          `${rate(row.cloned, row.lines).padStart(5)}%  ${row.group}`,
      );
    }
    if (options.top > 0) {
      console.log(`  -- the ${options.top} files with the most cloned lines`);
      const top = [...result.perFile]
        .sort((a, b) => b.cloned - a.cloned || (a.file < b.file ? -1 : 1))
        .slice(0, options.top);
      for (const entry of top) {
        console.log(
          `  ${String(entry.cloned).padStart(6)} / ${String(entry.lines).padStart(6)}  ` +
            `${rate(entry.cloned, entry.lines).padStart(5)}%  ${entry.file}`,
        );
      }
    }
  }
}

main();
