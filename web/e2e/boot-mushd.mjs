// Doc 43 cut TS.9 (D-43-8, §10.1): boot a real `mushd` for the browser smoke.
//
// The smoke drives the SPA against the binary the `rest-mushd` cell already
// built, not a Rust rig: what it exists to catch is the seam no Rust test
// crosses — the bundle `web/dist` served by `mush-net-web`, the socket the
// browser opens, the picture module the page loads. So the server is the
// production root (`mushd --config`), on the in-memory store (nothing to
// provision, nothing survives), with both listeners on loopback ports picked
// here and a scratch `data_dir` (host key, offer and resolution keys) removed
// afterwards.
//
// Used as Playwright's `globalSetup` (`playwright.config.ts`); the returned
// function is the teardown. Runnable alone too — `node e2e/boot-mushd.mjs`
// boots one and holds it until Ctrl+C, for poking at the page by hand.
import { spawn } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

/** A cold debug binary seeds the world, the starter area and the proving
 * ground before it listens; a loaded CI box is slower than a laptop. */
const BOOT_TIMEOUT_MS = 120_000;

/** `MUSHD_BIN`, else the workspace's debug binary (`mushd.exe` on Windows). */
function binary() {
  const fromEnv = process.env.MUSHD_BIN;
  if (fromEnv) return resolve(fromEnv);
  const exe = process.platform === "win32" ? "mushd.exe" : "mushd";
  const targetDir = process.env.CARGO_TARGET_DIR ? resolve(repo, process.env.CARGO_TARGET_DIR) : join(repo, "target");
  return join(targetDir, "debug", exe);
}

/** A loopback port nobody holds right now. There is a window between closing
 * this probe and `mushd` binding it; on a CI runner nothing else is racing for
 * loopback ports, and a lost race fails the boot loudly, never silently. */
function freePort() {
  return new Promise((ok, fail) => {
    const srv = createServer();
    srv.unref();
    srv.on("error", fail);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      srv.close(() => (typeof addr === "object" && addr ? ok(addr.port) : fail(new Error("no port"))));
    });
  });
}

/** The config: the smallest one the production root boots, plus the arena.
 * `[tacsim].enabled` is what instantiates the simulator (mushd.toml.example,
 * 16b cut F.4) — without it the pods are a set with nothing behind them and no
 * page can ever be seated in a cockpit, which is where the picture is drawn. */
function configText(dataDir, sshPort, webPort) {
  // TOML basic strings take backslash escapes; a Windows path must not carry
  // any into the file (as `durable_process/cli_args.rs` does).
  const slash = (p) => p.replace(/\\/g, "/");
  return [
    "[server]",
    'name = "rustmush e2e"',
    `data_dir = "${slash(dataDir)}"`,
    `help_dir = "${slash(join(repo, "docs", "help"))}"`,
    "[store]",
    'store = "memory"',
    "[ssh]",
    `listen = "127.0.0.1:${sshPort}"`,
    "[web]",
    `listen = "127.0.0.1:${webPort}"`,
    "[tacsim]",
    "enabled = true",
    "",
  ].join("\n");
}

async function waitHealthy(base, child, logPath) {
  const deadline = Date.now() + BOOT_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`mushd exited ${child.exitCode} before /healthz answered:\n${tail(logPath)}`);
    }
    try {
      const res = await fetch(`${base}/healthz`);
      if (res.ok) {
        const body = await res.json();
        if (body.ok === true) return;
      }
    } catch {
      // Not listening yet.
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`mushd did not answer /healthz within ${BOOT_TIMEOUT_MS} ms:\n${tail(logPath)}`);
}

function tail(path, lines = 60) {
  try {
    return readFileSync(path, "utf8").split("\n").slice(-lines).join("\n");
  } catch {
    return "(no log)";
  }
}

/** Boot one `mushd`; resolves once `/healthz` says ok. Sets `MUSHD_BASE_URL`
 * (and `MUSHD_LOG`) in the environment, which Playwright's workers inherit,
 * and returns the teardown. */
export async function bootMushd() {
  const bin = binary();
  if (!existsSync(bin)) {
    throw new Error(`no mushd binary at ${bin}: run \`cargo build -p mushd\` or set MUSHD_BIN`);
  }
  if (!existsSync(join(repo, "web", "dist", "index.html"))) {
    // Debug builds read `web/dist` from disk at run time and answer
    // "web client not built" without it; say why up front instead.
    throw new Error("no web/dist/index.html: run `npm run build` in web/ first");
  }
  const scratch = mkdtempSync(join(tmpdir(), "mush-e2e-"));
  const dataDir = join(scratch, "data");
  mkdirSync(dataDir, { recursive: true });
  const [sshPort, webPort] = [await freePort(), await freePort()];
  const configPath = join(scratch, "mushd.toml");
  writeFileSync(configPath, configText(dataDir, sshPort, webPort));
  const logPath = join(scratch, "mushd.log");
  const log = createWriteStream(logPath);

  const child = spawn(bin, ["--config", configPath], {
    cwd: repo,
    stdio: ["ignore", "pipe", "pipe"],
    // A level the log can be read at after a failure, without the per-frame
    // noise of debug.
    env: { ...process.env, RUST_LOG: process.env.RUST_LOG ?? "info" },
  });
  child.stdout.pipe(log);
  child.stderr.pipe(log);

  const base = `http://127.0.0.1:${webPort}`;
  const teardown = async () => {
    if (child.exitCode === null) {
      const exited = new Promise((r) => child.once("exit", r));
      child.kill();
      await Promise.race([exited, new Promise((r) => setTimeout(r, 10_000))]);
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    }
    log.end();
    if (process.env.MUSHD_KEEP_LOG) {
      console.log(`mushd log kept at ${logPath}`);
    } else {
      rmSync(scratch, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    }
  };
  try {
    await waitHealthy(base, child, logPath);
  } catch (err) {
    await teardown();
    throw err;
  }
  process.env.MUSHD_BASE_URL = base;
  process.env.MUSHD_LOG = logPath;
  return teardown;
}

export default bootMushd;

// `node e2e/boot-mushd.mjs`: boot one and hold it.
if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const teardown = await bootMushd();
  console.log(`mushd up at ${process.env.MUSHD_BASE_URL} (log ${process.env.MUSHD_LOG}); Ctrl+C stops it`);
  process.on("SIGINT", () => void teardown().then(() => process.exit(0)));
}
