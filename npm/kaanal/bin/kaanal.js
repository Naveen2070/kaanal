#!/usr/bin/env node
// Runs the kaanal binary from the platform package npm installed next to this one
// (scripts/npm-pack.mjs lists them as optionalDependencies and gives them an `os` and `cpu`).
"use strict";

const { spawnSync } = require("node:child_process");
const { chmodSync } = require("node:fs");

const SCOPE = "@kaanal";
const key = `${process.platform}-${process.arch}`;
const exe = process.platform === "win32" ? "kaanal.exe" : "kaanal";

let bin;
try {
  bin = require.resolve(`${SCOPE}/${key}/bin/${exe}`);
} catch {
  console.error(
    `kaanal has no build for ${key}. Download one from https://github.com/Naveen2070/kaanal/releases, ` +
      "or build it with cargo (cargo install kaanal). " +
      "If you installed with --no-optional, reinstall without it.",
  );
  process.exit(1);
}
try {
  chmodSync(bin, 0o755); // some installers drop the executable bit
} catch {
  // read-only install: it usually has the bit already
}

const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (r.error) {
  console.error(`kaanal: could not run ${bin}: ${r.error.message}`);
  process.exit(1);
}
process.exit(r.status ?? 1);
