#!/usr/bin/env node
// One command for UI development: build the binary, start a blank `kaanal`, start the Vite dev
// server with its /__kaanal proxy pointed at that kaanal, then print ONE url that carries the
// control token. The token is the whole point: `kaanal studio` prints it on the server's port,
// the page is served on the dev server's port, and this script moves it across.
//
//   node scripts/studio-dev.mjs            from the repo root
//   pnpm dev:studio                        from ui/
//
//   --no-build     use the binary already in target/debug (skip cargo)
//   --fresh        wipe the scratch project first (default: keep it, so work survives restarts)
//   --dir PATH     scratch project folder (default: <tmp>/kaanal-studio-dev)
//   --port N       kaanal port (default 7777)
//   --ui-port N    Vite dev port (default 5173)
//
// Ctrl+C stops both processes.

import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const WIN = process.platform === "win32";
const EXE = path.join(ROOT, "target", "debug", WIN ? "kaanal.exe" : "kaanal");

function parseArgs(argv) {
  const opts = {
    build: true,
    fresh: false,
    dir: path.join(os.tmpdir(), "kaanal-studio-dev"),
    port: 7777,
    uiPort: 5173,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--no-build") opts.build = false;
    else if (a === "--fresh") opts.fresh = true;
    else if (a === "--dir") opts.dir = path.resolve(argv[++i]);
    else if (a === "--port") opts.port = Number(argv[++i]);
    else if (a === "--ui-port") opts.uiPort = Number(argv[++i]);
    else if (a === "--help" || a === "-h") {
      console.log(
        "usage: studio-dev.mjs [--no-build] [--fresh] [--dir PATH] [--port N] [--ui-port N]",
      );
      process.exit(0);
    } else {
      console.error(`studio-dev: unknown argument ${a} (try --help)`);
      process.exit(2);
    }
  }
  for (const [name, n] of [
    ["port", opts.port],
    ["ui-port", opts.uiPort],
  ]) {
    if (!Number.isInteger(n) || n < 1 || n > 65535) {
      console.error(`studio-dev: --${name} must be a port number`);
      process.exit(2);
    }
  }
  return opts;
}

/**
 * True when nothing is listening on the port on either loopback family. Checking only 127.0.0.1
 * would call an ::1-only listener free, and the child would then fail to bind.
 */
function portFree(port) {
  const freeOn = (host) =>
    new Promise((resolve) => {
      const srv = net.createServer();
      srv.once("error", () => resolve(false));
      srv.once("listening", () => srv.close(() => resolve(true)));
      srv.listen(port, host);
    });
  return freeOn("127.0.0.1").then((v4) => (v4 ? freeOn("::1") : false));
}

/**
 * Quotes one argument for cmd.exe. Only needed on Windows, where the command is joined into a
 * single string; without this a path with spaces (a repo under `C:\Users\Jane Doe\...`) is split
 * into several arguments.
 */
function cmdQuote(arg) {
  const s = String(arg);
  return /[\s"&|<>^()%!]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/**
 * Spawns a child in its own process group, so the whole tree (pnpm -> vp -> vite) can be killed:
 * on POSIX a signal to the group does it, on Windows `taskkill /T` walks the tree.
 *
 * The two platforms genuinely differ here, and getting it wrong is silent:
 * - POSIX takes a real argv with no shell. No quoting, and a repo path with spaces just works.
 * - Windows needs a shell to find the `.cmd` shims (`pnpm` is `pnpm.cmd`, which cannot be spawned
 *   directly), and Node warns (DEP0190) about an args ARRAY alongside a shell — so the argv is
 *   joined into one quoted string for cmd.exe. Passing that string on POSIX would fail: with no
 *   shell, Node treats the whole string as a filename.
 */
function spawnTree(cmd, args, opts = {}) {
  const base = { stdio: ["ignore", "pipe", "inherit"], detached: !WIN, ...opts };
  if (WIN) return spawn([cmd, ...args].map(cmdQuote).join(" "), { ...base, shell: true });
  return spawn(cmd, args, base);
}

function killTree(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  if (WIN) {
    spawnSync("taskkill", ["/pid", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  } else {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {
      child.kill("SIGTERM");
    }
  }
}

/** Reads a child's stdout line by line, calling `onLine` for each. */
function pipeLines(child, onLine) {
  let buf = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    buf += chunk;
    let nl;
    while ((nl = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      onLine(line);
    }
  });
}

/**
 * Waits until something answers on the port, or throws after `ms`. Tries both loopback families:
 * a dev server bound to `localhost` may listen on ::1 only (Windows resolves it first), so a
 * check hardcoded to 127.0.0.1 fails against a perfectly healthy server.
 */
async function waitForServer(port, ms) {
  const until = Date.now() + ms;
  const tryHost = (host) =>
    new Promise((resolve) => {
      const sock = net.connect({ port, host });
      sock.once("connect", () => {
        sock.destroy();
        resolve(true);
      });
      sock.once("error", () => resolve(false));
    });
  for (;;) {
    if ((await tryHost("127.0.0.1")) || (await tryHost("::1"))) return;
    if (Date.now() > until) {
      throw new Error(`nothing answered on localhost:${port} within ${ms} ms`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

const opts = parseArgs(process.argv.slice(2));

for (const [name, port] of [
  ["kaanal", opts.port],
  ["Vite dev server", opts.uiPort],
]) {
  if (!(await portFree(port))) {
    console.error(`studio-dev: the ${name} port ${port} is already in use.`);
    console.error(`  Pass --port / --ui-port, or stop whatever holds it.`);
    process.exit(1);
  }
}

if (opts.build) {
  console.log(`studio-dev: cargo build -p kaanal`);
  const args = ["build", "-p", "kaanal"];
  const built = WIN
    ? spawnSync(["cargo", ...args].map(cmdQuote).join(" "), { cwd: ROOT, shell: true, stdio: "inherit" })
    : spawnSync("cargo", args, { cwd: ROOT, stdio: "inherit" });
  if (built.status !== 0) {
    console.error("studio-dev: cargo build failed; nothing started.");
    process.exit(built.status ?? 1);
  }
}

if (opts.fresh) rmSync(opts.dir, { recursive: true, force: true });
mkdirSync(opts.dir, { recursive: true });

// --- the backend: a blank kaanal, no browser ---
const kaanal = spawnTree(EXE, ["studio", "--no-open", "--port", String(opts.port)], {
  cwd: opts.dir,
});

let token = null;
const tokenReady = new Promise((resolve, reject) => {
  const timer = setTimeout(
    () => reject(new Error("kaanal did not print its URL within 30 s")),
    30_000,
  );
  pipeLines(kaanal, (line) => {
    console.log(`  kaanal | ${line}`);
    const m = /#token=([0-9a-fA-F]+)/.exec(line);
    if (m && !token) {
      token = m[1];
      clearTimeout(timer);
      resolve();
    }
  });
  kaanal.once("exit", (code) => {
    clearTimeout(timer);
    reject(new Error(`kaanal exited with code ${code} before it printed a token`));
  });
});

// --- the frontend: Vite, proxying /__kaanal at the kaanal above ---
const ui = spawnTree("pnpm", ["dev"], {
  cwd: path.join(ROOT, "ui"),
  stdio: "inherit",
  env: { ...process.env, KAANAL_DEV_PORT: String(opts.port) },
});

let stopping = false;
function stop(code) {
  if (stopping) return;
  stopping = true;
  killTree(ui);
  killTree(kaanal);
  process.exit(code);
}

ui.once("exit", (code) => {
  if (!stopping) {
    console.error(`studio-dev: the dev server exited (code ${code}); stopping kaanal.`);
    stop(code ?? 1);
  }
});
kaanal.once("exit", (code) => {
  if (!stopping) {
    console.error(`studio-dev: kaanal exited (code ${code}); stopping the dev server.`);
    stop(code ?? 1);
  }
});
process.on("SIGINT", () => stop(0));
process.on("SIGTERM", () => stop(0));

try {
  await tokenReady;
  await waitForServer(opts.uiPort, 60_000);
} catch (e) {
  console.error(`studio-dev: ${e.message}`);
  stop(1);
}

const url = `http://localhost:${opts.uiPort}/#token=${token}`;
console.log("");
console.log("  Studio dev is up.");
console.log(`    Open:   ${url}`);
console.log(`    API:    kaanal on 127.0.0.1:${opts.port} (blank; Save writes ${path.join(opts.dir, "kaanal.yaml")})`);
console.log(`    Edit:   ui/src/** and the browser reloads. Ctrl+C stops both.`);
console.log("");
