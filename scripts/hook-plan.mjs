#!/usr/bin/env node
// Plans the pre-commit checks from the staged files (stdin, one path a line), so the hook runs only
// what the change can affect. Prints one line per step for .githooks/pre-commit:
//
//   rust yes|no        the Rust files changed (fmt runs)
//   clippy <cargo args> a clippy run over these crates (the hook adds --all-targets)
//   test <cargo args>   a cargo test run: the changed crates and every crate that depends on them, or
//                       one integration-test binary when only that test's files changed
//   ui none|changed|all the UI changed: `changed` runs the tests related to the changed files, `all`
//                       runs them all (config, lockfile, anything outside ui/src)
//   docs yes|no         markdown changed (dprint runs)
//   note <text>         what the plan is, for the log
//
// A change that can reach everything (Cargo.toml, Cargo.lock, rustfmt.toml, the toolchain, examples/)
// plans the whole workspace. The CI runs everything regardless; this only keeps a commit quick.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

const files = readFileSync(0, "utf8")
  .split("\n")
  .map((s) => s.trim().replace(/\\/g, "/"))
  .filter(Boolean);

const out = [];
const say = (line) => out.push(line);

// ---- what kind of change
const workspaceWide = (f) =>
  /^(Cargo\.(toml|lock)|rustfmt\.toml|rust-toolchain(\.toml)?|\.cargo\/|examples\/)/.test(f);
const rustFiles = files.filter((f) => f.startsWith("crates/") || workspaceWide(f));
const uiFiles = files.filter((f) => f.startsWith("ui/"));
const docsFiles = files.filter((f) => f.endsWith(".md") && !f.startsWith("internal/"));

// ---- Rust
if (rustFiles.length === 0) {
  say("rust no");
} else {
  say("rust yes");
  if (rustFiles.some(workspaceWide)) {
    say("note Rust: the whole workspace (a workspace-level file changed)");
    say("clippy --workspace");
    say("test --workspace");
  } else {
    const meta = JSON.parse(
      execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
        encoding: "utf8",
        maxBuffer: 1 << 28,
      }),
    );
    const root = meta.workspace_root.replace(/\\/g, "/");
    const pkgs = meta.packages.map((p) => ({
      name: p.name,
      dir: path.dirname(p.manifest_path).replace(/\\/g, "/").replace(`${root}/`, ""),
      deps: p.dependencies.map((d) => d.name),
    }));
    const wide = new Set(); // crates whose own code (or anything not a test file) changed
    const bins = new Map(); // crate -> integration-test binaries whose files changed
    for (const f of rustFiles) {
      const pkg = pkgs.find((p) => f.startsWith(`${p.dir}/`));
      if (!pkg) {
        wide.add("*");
        continue;
      }
      const rest = f.slice(pkg.dir.length + 1);
      const parts = rest.split("/");
      let bin = null;
      if (parts[0] === "tests") {
        // tests/name.rs is its own binary; tests/dir/... belongs to the binary `dir` when it has a main.rs
        if (parts.length === 2 && parts[1].endsWith(".rs")) bin = parts[1].slice(0, -3);
        else if (parts.length >= 3 && existsSync(path.join(root, pkg.dir, "tests", parts[1], "main.rs")))
          bin = parts[1];
      }
      if (bin) {
        if (!bins.has(pkg.name)) bins.set(pkg.name, new Set());
        bins.get(pkg.name).add(bin);
      } else {
        wide.add(pkg.name);
      }
    }
    if (wide.has("*")) {
      say("note Rust: the whole workspace (a file outside every crate)");
      say("clippy --workspace");
      say("test --workspace");
    } else {
      // every crate that depends on a changed one, however far down
      const affected = new Set(wide);
      for (let grew = true; grew; ) {
        grew = false;
        for (const p of pkgs) {
          if (!affected.has(p.name) && p.deps.some((d) => affected.has(d))) {
            affected.add(p.name);
            grew = true;
          }
        }
      }
      const only = [...bins.entries()].filter(([name]) => !affected.has(name));
      const names = [...affected, ...only.map(([n]) => n)];
      say(
        `note Rust: ${[...affected].join(", ") || "no crate-wide change"}` +
          (affected.size > wide.size
            ? ` (${[...wide].join(", ")} changed; the rest depend on it)`
            : "") +
          only.map(([n, b]) => `; ${n} tests ${[...b].join(", ")} only`).join(""),
      );
      say(`clippy ${names.map((n) => `-p ${n}`).join(" ")}`);
      if (affected.size > 0) say(`test ${[...affected].map((n) => `-p ${n}`).join(" ")}`);
      for (const [name, set] of only) {
        say(`test -p ${name} ${[...set].map((b) => `--test ${b}`).join(" ")}`);
      }
    }
  }
}

// ---- UI
if (uiFiles.length === 0) {
  say("ui none");
} else {
  // a change to a config, the lockfile or a script can move any test; source files have their own
  const broad = uiFiles.some((f) => !f.startsWith("ui/src/"));
  say(`ui ${broad ? "all" : "changed"}`);
}

// ---- docs
say(`docs ${docsFiles.length > 0 ? "yes" : "no"}`);

process.stdout.write(`${out.join("\n")}\n`);
