#!/usr/bin/env node
// Builds the npm packages of a release from the archives release.yml builds (RULES 148 d).
//
// The pattern of esbuild and biome: one package per platform holding only the binary (npm installs
// the one whose `os` and `cpu` match), and a wrapper `kaanal` that lists them as optionalDependencies
// and runs the right one (npm/kaanal/bin/kaanal.js). Nothing is downloaded at install time, so
// `--ignore-scripts` and proxies do not matter.
//
// Usage: node scripts/npm-pack.mjs --tag v0.1.0 --archives DIR --out DIR
//   DIR/kaanal-<tag>-<target>.tar.gz|zip in, OUT/<platform>/ and OUT/kaanal/ out, ready for `npm publish`.
// Publish the platform packages first, then OUT/kaanal.

import { execFileSync } from "node:child_process";
import { chmodSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The npm scope of the platform packages; the launcher in npm/kaanal/bin/kaanal.js names the same one. */
export const SCOPE = "@kaanal";

/** One entry per archive release.yml builds. `key` is `process.platform-process.arch`. */
export const PLATFORMS = [
  { target: "x86_64-unknown-linux-gnu", key: "linux-x64", os: "linux", cpu: "x64", ext: "tar.gz", exe: "kaanal" },
  { target: "aarch64-apple-darwin", key: "darwin-arm64", os: "darwin", cpu: "arm64", ext: "tar.gz", exe: "kaanal" },
  { target: "x86_64-pc-windows-msvc", key: "win32-x64", os: "win32", cpu: "x64", ext: "zip", exe: "kaanal.exe" },
];

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const common = {
  license: "Apache-2.0",
  homepage: "https://kaanal.naveenr.in",
  repository: { type: "git", url: "git+https://github.com/Naveen2070/kaanal.git" },
};

// GNU tar on Windows reads "C:" in a path as a remote host.
export const local = process.platform === "win32" ? ["--force-local"] : [];

function extract(archive, into, ext) {
  mkdirSync(into, { recursive: true });
  if (ext === "zip") execFileSync("unzip", ["-q", archive], { cwd: into });
  else execFileSync("tar", [...local, "-xzf", archive], { cwd: into });
}

/** Writes the packages and returns their directories, platform packages first and the wrapper last. */
export function build({ tag, archives, out }) {
  if (!/^v\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(tag)) throw new Error(`bad tag: ${tag}`);
  const version = tag.slice(1);
  out = resolve(out);
  rmSync(out, { recursive: true, force: true });
  const dirs = [];
  for (const p of PLATFORMS) {
    const archive = resolve(archives, `kaanal-${tag}-${p.target}.${p.ext}`); // tar runs in another directory
    if (!existsSync(archive)) throw new Error(`missing archive: ${archive}`);
    const scratch = join(out, `.unpack-${p.key}`);
    extract(archive, scratch, p.ext);
    const dir = join(out, p.key);
    mkdirSync(join(dir, "bin"), { recursive: true });
    const bin = join(dir, "bin", p.exe);
    cpSync(join(scratch, `kaanal-${tag}-${p.target}`, p.exe), bin);
    if (p.os !== "win32") chmodSync(bin, 0o755);
    cpSync(join(root, "LICENSE"), join(dir, "LICENSE"));
    rmSync(scratch, { recursive: true, force: true });
    writeFileSync(
      join(dir, "package.json"),
      JSON.stringify(
        {
          name: `${SCOPE}/${p.key}`,
          version,
          description: `The kaanal binary for ${p.os} ${p.cpu}. Install \`kaanal\` instead.`,
          os: [p.os],
          cpu: [p.cpu],
          files: ["bin/", "LICENSE"],
          ...common,
        },
        null,
        2,
      ) + "\n",
    );
    dirs.push(dir);
  }
  const wrapper = join(out, "kaanal");
  cpSync(join(root, "npm", "kaanal"), wrapper, { recursive: true });
  cpSync(join(root, "LICENSE"), join(wrapper, "LICENSE"));
  writeFileSync(
    join(wrapper, "package.json"),
    JSON.stringify(
      {
        name: "kaanal",
        version,
        description: "Kaanal: a mirage of your backend. Mock APIs, chaos, and what-if simulation in one binary.",
        keywords: ["mock", "openapi", "chaos", "simulation", "api"],
        bin: { kaanal: "bin/kaanal.js" },
        files: ["bin/", "README.md", "LICENSE"],
        engines: { node: ">=18" },
        optionalDependencies: Object.fromEntries(PLATFORMS.map((p) => [`${SCOPE}/${p.key}`, version])),
        ...common,
      },
      null,
      2,
    ) + "\n",
  );
  dirs.push(wrapper);
  return dirs;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const arg = (name) => {
    const i = process.argv.indexOf(`--${name}`);
    if (i < 0 || !process.argv[i + 1]) throw new Error(`--${name} is required`);
    return process.argv[i + 1];
  };
  for (const d of build({ tag: arg("tag"), archives: arg("archives"), out: arg("out") })) console.log(d);
}
