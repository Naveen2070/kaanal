// `node --test scripts/hook-plan.test.mjs`: the pre-commit planner picks the narrowest tests the change can reach.
// Runs against the real workspace (cargo metadata), so it picks its example files from the tree instead of naming them.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";

const plan = (files, env = {}) =>
  execFileSync("node", ["scripts/hook-plan.mjs"], {
    input: files.join("\n"),
    encoding: "utf8",
    env: { ...process.env, KAANAL_HOOK: "", ...env },
  })
    .trim()
    .split("\n");

// a top-level src file of kaanal-sim that has unit tests of its own
const unitFile = readdirSync("crates/kaanal-sim/src")
  .filter((f) => f.endsWith(".rs") && !["lib.rs", "main.rs"].includes(f))
  .find((f) => /#\[(cfg\(test\)|test)/.test(readFileSync(`crates/kaanal-sim/src/${f}`, "utf8")));
const testFile = readdirSync("crates/kaanal-core/tests").find((f) => f.endsWith(".rs"));

test("a src file with unit tests runs that module only; dependents are linted, not tested", () => {
  assert.ok(unitFile, "kaanal-sim should have a src file with unit tests");
  const mod = unitFile.slice(0, -3);
  const p = plan([`crates/kaanal-sim/src/${unitFile}`]);
  assert.ok(p.includes("rust yes"));
  assert.deepEqual(p.filter((l) => l.startsWith("test ")), [`test -p kaanal-sim --lib -- ${mod}`]);
  assert.match(p.find((l) => l.startsWith("clippy ")), /-p kaanal-sim .*-p kaanal-server/);
});

test("KAANAL_HOOK=crate runs the whole crate, deps adds the crates that depend on it", () => {
  const f = [`crates/kaanal-sim/src/${unitFile}`];
  assert.deepEqual(plan(f, { KAANAL_HOOK: "crate" }).filter((l) => l.startsWith("test ")), ["test -p kaanal-sim"]);
  const deps = plan(f, { KAANAL_HOOK: "deps" }).filter((l) => l.startsWith("test "));
  assert.ok(deps.includes("test -p kaanal-sim") && deps.includes("test -p kaanal-server"));
});

test("a changed integration-test file runs only that binary", () => {
  const bin = testFile.slice(0, -3);
  const t = plan([`crates/kaanal-core/tests/${testFile}`]).filter((l) => l.startsWith("test "));
  assert.deepEqual(t, [`test -p kaanal-core --test ${bin}`]);
});

test("several files in one plan add up", () => {
  const t = plan([`crates/kaanal-sim/src/${unitFile}`, `crates/kaanal-core/tests/${testFile}`]).filter((l) =>
    l.startsWith("test "),
  );
  assert.equal(t.length, 2);
});

test("scripts and example shell files are not Rust changes", () => {
  assert.ok(plan(["examples/nginx-server/check.sh", "README.md"]).includes("rust no"));
  assert.ok(plan(["Cargo.lock"]).includes("test --workspace"));
  assert.ok(plan(["examples/nginx-server/kaanal.yaml"]).includes("test --workspace"));
});

test("the UI runs the tests related to the staged files; a config change runs them all", () => {
  const some = plan(["ui/src/token.ts"]);
  assert.ok(some.includes("ui changed") && some.includes("uifiles src/token.ts"));
  assert.ok(plan(["ui/package.json"]).includes("ui all"));
});

test("a script with a test next to it plans that test", () => {
  assert.ok(plan(["scripts/hook-plan.mjs"]).includes("node-test scripts/hook-plan.test.mjs"));
  assert.ok(plan([".githooks/pre-commit"]).includes("node-test scripts/hook-plan.test.mjs"));
});

test("a lock file change plans the third-party licenses check, nothing else does", () => {
  assert.ok(plan(["Cargo.lock"]).includes("licenses yes"));
  assert.ok(plan(["ui/pnpm-lock.yaml"]).includes("licenses yes"));
  assert.ok(plan(["README.md"]).includes("licenses no"));
});
