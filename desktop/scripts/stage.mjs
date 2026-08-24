#!/usr/bin/env node
// Cross-platform twin of stage.sh: stages what the desktop app bundles for
// first-launch provisioning. The nurb wheel built from this checkout, a fully
// pinned hash-locked resolution of its dependencies, the committed adapter
// manifest/lock, and the uv sidecar binaries for the targets this platform
// builds (both darwin triples on macOS, the native triple on Windows). Runs
// before every tauri dev/build; the uv downloads are skipped once present.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const tauri = path.join(here, "..", "src-tauri");
const repo = path.join(here, "..", "..");
const adapterRuntime = path.join(here, "..", "adapter-runtime");
const resources = path.join(tauri, "resources");
const binaries = path.join(tauri, "binaries");
const UV_VERSION = "0.12.1";

fs.mkdirSync(resources, { recursive: true });
fs.mkdirSync(binaries, { recursive: true });

for (const name of fs.readdirSync(resources)) {
  if (name.startsWith("nurb-") && name.endsWith(".whl")) {
    fs.rmSync(path.join(resources, name));
  }
}
execFileSync("uv", ["build", "--wheel", "--project", repo, "-o", resources], {
  stdio: ["ignore", "ignore", "inherit"],
});
execFileSync(
  "uv",
  [
    "pip",
    "compile",
    path.join(repo, "pyproject.toml"),
    "--universal",
    "--python-version",
    "3.13",
    "--generate-hashes",
    "--no-annotate",
    "-q",
    "-o",
    path.join(resources, "requirements.lock"),
  ],
  { stdio: "inherit" },
);
fs.copyFileSync(
  path.join(adapterRuntime, "package.json"),
  path.join(resources, "adapter-package.json"),
);
fs.copyFileSync(
  path.join(adapterRuntime, "package-lock.json"),
  path.join(resources, "adapter-package-lock.json"),
);

// Tauri resolves externalBin per target triple, so each platform only needs
// the sidecars it can build.
const windows = process.platform === "win32";
const triples = windows
  ? [os.arch() === "arm64" ? "aarch64-pc-windows-msvc" : "x86_64-pc-windows-msvc"]
  : ["aarch64-apple-darwin", "x86_64-apple-darwin"];

for (const triple of triples) {
  const out = path.join(binaries, windows ? `uv-${triple}.exe` : `uv-${triple}`);
  if (fs.existsSync(out)) continue;
  console.log(`stage: downloading uv ${UV_VERSION} for ${triple}`);
  const archiveName = windows ? `uv-${triple}.zip` : `uv-${triple}.tar.gz`;
  const base = `https://github.com/astral-sh/uv/releases/download/${UV_VERSION}`;
  const archive = await fetched(`${base}/${archiveName}`);
  const published = (await fetched(`${base}/${archiveName}.sha256`))
    .toString("utf8")
    .split(/\s+/)[0]
    .toLowerCase();
  const actual = createHash("sha256").update(archive).digest("hex");
  if (actual !== published) {
    console.error(`stage: checksum mismatch for ${archiveName}`);
    process.exit(1);
  }
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "nurb-stage-"));
  const archivePath = path.join(scratch, archiveName);
  fs.writeFileSync(archivePath, archive);
  // Both OSes ship bsdtar as their system tar; -xf auto-detects zip and gz.
  execFileSync(systemTar(), ["-xf", archivePath, "-C", scratch], { stdio: "inherit" });
  const found = findFile(scratch, windows ? "uv.exe" : "uv");
  if (!found) {
    console.error("stage: uv binary not found in archive");
    process.exit(1);
  }
  fs.copyFileSync(found, out);
  if (!windows) fs.chmodSync(out, 0o755);
  fs.rmSync(scratch, { recursive: true, force: true });
}

function systemTar() {
  if (!windows) return "/usr/bin/tar";
  return path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe");
}

async function fetched(url) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) {
    console.error(`stage: download failed (${response.status}) for ${url}`);
    process.exit(1);
  }
  return Buffer.from(await response.arrayBuffer());
}

function findFile(root, name) {
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const full = path.join(root, entry.name);
    if (entry.isDirectory()) {
      const nested = findFile(full, name);
      if (nested) return nested;
    } else if (entry.name === name) {
      return full;
    }
  }
  return null;
}
