import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { sourceCommit } from './stamp-commit.mjs';

test('archive marker works without git, including rollback commits', () => {
  const root = mkdtempSync(resolve(tmpdir(), 'rustmush-bundle-'));
  try {
    for (const commit of ['a'.repeat(40), '0'.repeat(40)]) {
      writeFileSync(resolve(root, '.source-commit'), `${commit}\n`);
      assert.equal(sourceCommit(root), commit);
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('checkout marker resolves actual HEAD', () => {
  const root = resolve(import.meta.dirname, '../..');
  assert.equal(sourceCommit(root), execFileSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim());
});
