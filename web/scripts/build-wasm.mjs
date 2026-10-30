// Doc 21 cut G.3b: build the cockpit picture's WebAssembly module
// (`crates/mush-pixel-wasm`, plain exports, no wasm-bindgen) and copy it to
// `web/src/lib/pixel/mush_pixel.wasm`, where Vite fingerprints it into the
// bundle and the parity test reads it. Doc 22 cut U.6: the unit pack
// (`crates/mush-pixel/looks/units.bin`, which the module does not embed —
// rule 7's budget) goes beside it as `units.bin`, fetched by the page. Run before `vite build` and before
// the tests; it needs the Rust toolchain and its `wasm32-unknown-unknown`
// target (`rustup target add wasm32-unknown-unknown`), never wasm-bindgen.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, statSync } from 'node:fs';
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
const units = join(root, 'web', 'src', 'lib', 'pixel', 'units.bin');
copyFileSync(join(root, 'crates', 'mush-pixel', 'looks', 'units.bin'), units);
console.log(`Unit pack: ${units} (${statSync(units).size} bytes)`);
