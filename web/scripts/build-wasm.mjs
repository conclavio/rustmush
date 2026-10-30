// Doc 21 cut G.3b: build the cockpit picture's WebAssembly module
// (`crates/mush-pixel-wasm`, plain exports, no wasm-bindgen) and copy it to
// `web/src/lib/pixel/mush_pixel.wasm`, where Vite fingerprints it into the
// bundle and the parity test reads it. Doc 31 cut UD.1: the model files
// (`crates/mush-pixel/models/*.mupk`, which the module does not embed) go
// beside it under `models/`, each fetched by the page when first needed. Run before `vite build` and before
// the tests; it needs the Rust toolchain and its `wasm32-unknown-unknown`
// target (`rustup target add wasm32-unknown-unknown`), never wasm-bindgen.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readdirSync, rmSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const target = 'wasm32-unknown-unknown';
execFileSync(
  'cargo',
  ['build', '--release', '--locked', '--target', target, '-p', 'mush-pixel-wasm'],
  { cwd: root, stdio: 'inherit' },
);
const targetDir = process.env.CARGO_TARGET_DIR
  ? resolve(root, process.env.CARGO_TARGET_DIR)
  : join(root, 'target');
const built = join(targetDir, target, 'release', 'mush_pixel_wasm.wasm');
const out = join(root, 'web', 'src', 'lib', 'pixel', 'mush_pixel.wasm');
mkdirSync(dirname(out), { recursive: true });
copyFileSync(built, out);
console.log(`Picture module: ${out} (${statSync(out).size} bytes)`);
// Doc 31 cut UD.1 (D-31-2, D-31-11): each model's file goes into `models/`
// beside the module, where Vite fingerprints it into `assets/` and the page
// fetches it only when a unit drawn from it is in a feed.
const modelsOut = join(root, 'web', 'src', 'lib', 'pixel', 'models');
rmSync(modelsOut, { recursive: true, force: true });
mkdirSync(modelsOut, { recursive: true });
const modelsIn = join(root, 'crates', 'mush-pixel', 'models');
let total = 0;
const files = readdirSync(modelsIn).filter((f) => f.endsWith('.mupk'));
for (const f of files) {
  copyFileSync(join(modelsIn, f), join(modelsOut, f));
  total += statSync(join(modelsOut, f)).size;
}
console.log(`Unit models: ${files.length} files in ${modelsOut} (${total} bytes)`);
// Doc 31 cut UD.2: the staff unit designer's catalogue — the shipped unit,
// weapon, sensor and core packs, the shipped looks and the model index —
// copied under `src/lib/designer/content/`, which the studio imports; the
// rules themselves are checked by the server, never here.
const designerOut = join(root, 'web', 'src', 'lib', 'designer', 'content');
rmSync(designerOut, { recursive: true, force: true });
const designerIn = [
  ['units', join(root, 'crates', 'tacsim', 'content', 'units')],
  ['weapons', join(root, 'crates', 'tacsim', 'content', 'weapons')],
  ['sensors', join(root, 'crates', 'tacsim', 'content', 'sensors')],
  ['cores', join(root, 'crates', 'tacsim', 'content', 'cores')],
  ['looks', join(root, 'crates', 'mush-pixel', 'looks')],
];
let designerFiles = 0;
for (const [name, dir] of designerIn) {
  mkdirSync(join(designerOut, name), { recursive: true });
  for (const f of readdirSync(dir).filter((f) => f.endsWith('.json'))) {
    copyFileSync(join(dir, f), join(designerOut, name, f));
    designerFiles += 1;
  }
}
copyFileSync(join(modelsIn, 'index.json'), join(designerOut, 'models.json'));
console.log(`Unit designer catalogue: ${designerFiles + 1} files in ${designerOut}`);
