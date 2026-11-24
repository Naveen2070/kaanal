// `node --test scripts/npm-pack.test.mjs`: builds the packages from fake archives and checks what npm will see.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { NOTICES, PLATFORMS, SCOPE, build, local } from "./npm-pack.mjs";

function fakeArchives(tag) {
  const dir = mkdtempSync(join(tmpdir(), "npm-pack-"));
  for (const p of PLATFORMS) {
    const name = `kaanal-${tag}-${p.target}`;
    mkdirSync(join(dir, "src", name), { recursive: true });
    writeFileSync(join(dir, "src", name, p.exe), `binary for ${p.key}`);
    const archive = join(dir, `${name}.${p.ext}`);
    if (p.ext === "zip") execFileSync("zip", ["-qr", archive, name], { cwd: join(dir, "src") });
    else execFileSync("tar", [...local, "-czf", archive, name], { cwd: join(dir, "src") });
  }
  return dir;
}

test("builds_one_package_per_platform_and_a_wrapper_that_depends_on_them_all", () => {
  const dir = fakeArchives("v1.2.3");
  const dirs = build({ tag: "v1.2.3", archives: dir, out: join(dir, "out") });
  assert.equal(dirs.length, PLATFORMS.length + 1);
  assert.ok(dirs.at(-1).endsWith("kaanal"), "the wrapper is published last");
  const json = (d) => JSON.parse(readFileSync(join(d, "package.json"), "utf8"));
  for (const [i, p] of PLATFORMS.entries()) {
    const pkg = json(dirs[i]);
    assert.equal(pkg.name, `${SCOPE}/${p.key}`);
    assert.equal(pkg.version, "1.2.3");
    assert.deepEqual([pkg.os, pkg.cpu], [[p.os], [p.cpu]]);
    assert.equal(readFileSync(join(dirs[i], "bin", p.exe), "utf8"), `binary for ${p.key}`);
    if (p.os !== "win32" && process.platform !== "win32") assert.ok(statSync(join(dirs[i], "bin", p.exe)).mode & 0o100, "executable");
    for (const f of NOTICES) assert.ok(statSync(join(dirs[i], f)).size > 0, `${f} is in ${p.key}`);
    assert.ok(pkg.files.includes("THIRD_PARTY_LICENSES.md"));
    assert.equal(pkg.repository.url, "git+https://github.com/Naveen2070/kaanal.git");
  }
  const w = json(dirs.at(-1));
  assert.equal(w.name, "kaanal");
  for (const f of NOTICES) assert.ok(statSync(join(dirs.at(-1), f)).size > 0, `${f} is in the wrapper`);
  assert.ok(w.files.includes("THIRD_PARTY_LICENSES.md"));
  assert.deepEqual(
    w.optionalDependencies,
    Object.fromEntries(PLATFORMS.map((p) => [`${SCOPE}/${p.key}`, "1.2.3"])),
  );
  assert.ok(readFileSync(join(dirs.at(-1), "bin", "kaanal.js"), "utf8").includes(`const SCOPE = "${SCOPE}"`));
});

test("a_bad_tag_or_a_missing_archive_is_refused", () => {
  const dir = fakeArchives("v1.2.3");
  assert.throws(() => build({ tag: "1.2.3", archives: dir, out: join(dir, "o") }), /bad tag/);
  assert.throws(() => build({ tag: "v9.9.9", archives: dir, out: join(dir, "o") }), /missing archive/);
});
