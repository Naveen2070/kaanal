#!/usr/bin/env node
// Plans the pre-commit checks from the staged files (stdin, one path a line), so the hook runs only
// what the change can affect. Prints one line per step for .githooks/pre-commit:
//
//   rust yes|no          the Rust files changed (fmt runs)
//   clippy <cargo args>  a clippy run over the changed crates and the crates that depend on them (the
//                        hook adds --all-targets, so dependents are compiled with their tests)
//   test <cargo args>    one cargo test run, as narrow as the change allows (see "Rust tests")
//   ui none|changed|all  the UI changed: `changed` runs the tests related to the staged files (the
//                        `uifiles` line), `all` runs them all (config, lockfile, anything outside ui/src)
//   uifiles <paths>      the staged UI files, relative to ui/
//   docs yes|no          markdown changed (dprint runs)
//   node-test <file>     a node test for a staged script (scripts/x.mjs -> scripts/x.test.mjs)
//   note <text>          what the plan is, for the log
//
// Rust tests, by what changed (KAANAL_HOOK picks how wide; the whole thing is KAANAL_HOOK_FULL=1):
//   smart (default)  test files only        -> just those integration-test binaries
//                    a src file with tests  -> that module's unit tests (`--lib -- module::path`)
//                    a src file without     -> the integration tests whose name matches its module,
//                                              else that crate's whole suite
//                    dependents             -> compiled and linted (clippy), not tested
//   crate            the whole suite of every changed crate, no dependents
//   deps             crate, plus the whole suite of every crate that depends on a changed one
// A change that can reach everything (Cargo.toml, Cargo.lock, rustfmt.toml, the toolchain, example
// configs) plans the whole workspace. The CI runs everything regardless; this keeps a commit quick.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";

const mode = (process.env.KAANAL_HOOK || "smart").toLowerCase();
const files = readFileSync(0, "utf8")
  .split("\n")
  .map((s) => s.trim().replace(/\\/g, "/"))
  .filter(Boolean);

const out = [];
const say = (line) => out.push(line);

// ---- what kind of change
const workspaceWide = (f) =>
  /^(Cargo\.(toml|lock)|rustfmt\.toml|rust-toolchain(\.toml)?|\.cargo\/)/.test(f) ||
  (f.startsWith("examples/") && !/\.(sh|ps1|md|txt)$/.test(f));
const rustFiles = files.filter(
  (f) => (f.startsWith("crates/") && !f.endsWith(".md")) || workspaceWide(f),
);
const uiFiles = files.filter((f) => f.startsWith("ui/"));
const docsFiles = files.filter((f) => f.endsWith(".md") && !f.startsWith("internal/"));
const hasTests = /#\[(cfg\(test\)|test|tokio::test)/;

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
    const pkgs = meta.packages.map((p) => {
      const dir = path.dirname(p.manifest_path).replace(/\\/g, "/").replace(`${root}/`, "");
      return {
        name: p.name,
        dir,
        deps: p.dependencies.map((d) => d.name),
        lib: existsSync(path.join(root, dir, "src", "lib.rs")),
        main: existsSync(path.join(root, dir, "src", "main.rs")),
      };
    });
    const byName = new Map(pkgs.map((p) => [p.name, p]));

    // integration-test binaries of a crate: tests/x.rs is `x`; tests/dir/ is `dir` when it has a main.rs
    const binaries = (pkg) => {
      const dir = path.join(root, pkg.dir, "tests");
      if (!existsSync(dir)) return [];
      return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
        e.isFile() && e.name.endsWith(".rs")
          ? [e.name.slice(0, -3)]
          : e.isDirectory() && existsSync(path.join(dir, e.name, "main.rs"))
            ? [e.name]
            : [],
      );
    };

    // per crate: `wide` = run its whole suite; `units` = module filters for its unit tests;
    // `bins` = integration-test binaries to run
    const wide = new Set();
    const units = new Map();
    const bins = new Map();
    const add = (map, name, v) => {
      if (!map.has(name)) map.set(name, new Set());
      map.get(name).add(v);
    };
    const why = [];
    let outside = false;

    for (const f of rustFiles) {
      const pkg = pkgs.find((p) => f.startsWith(`${p.dir}/`));
      if (!pkg) {
        outside = true;
        continue;
      }
      const rest = f.slice(pkg.dir.length + 1);
      const parts = rest.split("/");
      if (parts[0] === "tests") {
        // tests/name.rs is its own binary; tests/dir/... belongs to the binary `dir` when it has a main.rs
        let bin = null;
        if (parts.length === 2 && parts[1].endsWith(".rs")) bin = parts[1].slice(0, -3);
        else if (parts.length >= 3 && existsSync(path.join(root, pkg.dir, "tests", parts[1], "main.rs")))
          bin = parts[1];
        if (bin) add(bins, pkg.name, bin);
        else wide.add(pkg.name);
      } else if (parts[0] === "src" && rest.endsWith(".rs") && !["src/lib.rs", "src/main.rs"].includes(rest)) {
        // src/sim/run.rs -> sim::run, src/sim/mod.rs -> sim
        const segs = rest.slice(4, -3).split("/");
        if (segs[segs.length - 1] === "mod") segs.pop();
        const file = path.join(root, f);
        if (existsSync(file) && hasTests.test(readFileSync(file, "utf8"))) {
          add(units, pkg.name, segs.join("::"));
        } else {
          // no tests in the file: the integration tests that carry its module name, else the whole crate
          const hit = binaries(pkg).filter((b) =>
            [segs[0], segs[segs.length - 1]].some((s) => b.toLowerCase().includes(s.toLowerCase())),
          );
          if (hit.length > 0) {
            hit.forEach((b) => add(bins, pkg.name, b));
            why.push(`${f}: no tests of its own, ran ${hit.join(", ")}`);
          } else {
            wide.add(pkg.name);
            why.push(`${f}: no tests found for it, ran the whole ${pkg.name} suite`);
          }
        }
      } else {
        wide.add(pkg.name); // lib.rs, main.rs, build.rs, a data file, the crate's Cargo.toml
      }
    }

    if (outside) {
      say("note Rust: the whole workspace (a file outside every crate)");
      say("clippy --workspace");
      say("test --workspace");
    } else {
      const changed = new Set([...wide, ...units.keys(), ...bins.keys()]);
      // every crate that depends on a changed one, however far down
      const reach = new Set(changed);
      for (let grew = true; grew; ) {
        grew = false;
        for (const p of pkgs) {
          if (!reach.has(p.name) && p.deps.some((d) => reach.has(d))) {
            reach.add(p.name);
            grew = true;
          }
        }
      }
      const dependents = [...reach].filter((n) => !changed.has(n));
      if (mode === "crate" || mode === "deps") for (const n of changed) wide.add(n);
      if (mode === "deps") for (const n of dependents) wide.add(n);

      const tests = [];
      for (const n of [...reach].filter((n) => wide.has(n))) tests.push(`-p ${n}`);
      for (const [n, set] of units) {
        if (wide.has(n)) continue;
        const p = byName.get(n);
        const targets = [p.lib ? "--lib" : "", p.main ? "--bins" : ""].filter(Boolean).join(" ");
        tests.push(`-p ${n} ${targets} -- ${[...set].join(" ")}`);
      }
      for (const [n, set] of bins) {
        if (wide.has(n)) continue;
        tests.push(`-p ${n} ${[...set].map((b) => `--test ${b}`).join(" ")}`);
      }

      const desc = [
        ...[...wide].filter((n) => changed.has(n)).map((n) => `${n} (whole suite)`),
        ...[...units].filter(([n]) => !wide.has(n)).map(([n, s]) => `${n} unit tests ${[...s].join(", ")}`),
        ...[...bins].filter(([n]) => !wide.has(n)).map(([n, s]) => `${n} tests ${[...s].join(", ")}`),
      ];
      say(
        `note Rust (${mode}): ${desc.join("; ") || "nothing to test"}` +
          (dependents.length
            ? mode === "deps"
              ? `; dependents tested too: ${dependents.join(", ")}`
              : `; dependents only linted: ${dependents.join(", ")} (KAANAL_HOOK=deps tests them)`
            : ""),
      );
      for (const w of why) say(`note   ${w}`);
      say(`clippy ${[...reach].map((n) => `-p ${n}`).join(" ")}`);
      for (const t of tests) say(`test ${t}`);
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
  say(`uifiles ${uiFiles.map((f) => f.slice(3)).join(" ")}`);
}

// ---- docs
say(`docs ${docsFiles.length > 0 ? "yes" : "no"}`);

// ---- node scripts that have a test next to them (and the hook itself, which has one)
const nodeTests = new Set();
for (const f of files) {
  if (f.endsWith(".test.mjs") && f.startsWith("scripts/")) nodeTests.add(f);
  else if (/^scripts\/[^/]+\.mjs$/.test(f) && existsSync(f.replace(/\.mjs$/, ".test.mjs")))
    nodeTests.add(f.replace(/\.mjs$/, ".test.mjs"));
  else if (f === ".githooks/pre-commit") nodeTests.add("scripts/hook-plan.test.mjs");
}
for (const t of nodeTests) say(`node-test ${t}`);

process.stdout.write(`${out.join("\n")}\n`);
