// git archive expands .source-commit; a checkout resolves its own HEAD.
// Stamp only after Vite succeeds, so a failed build cannot bless an old dist.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export function sourceCommit(root) {
  const marker = readFileSync(resolve(root, '.source-commit'), 'utf8').trim();
  const commit = /^[0-9a-f]{40,64}$/.test(marker)
    ? marker
    : execFileSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
  if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(commit)) {
    throw new Error('Cannot determine the source commit; build from a checkout or git archive.');
  }
  return commit;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
  const commit = sourceCommit(root);
  writeFileSync(resolve(root, 'web/dist/.commit'), `${commit}\n`);
  console.log(`Web bundle source commit: ${commit}`);
}
