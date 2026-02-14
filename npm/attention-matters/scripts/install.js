#!/usr/bin/env node

"use strict";

const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const https = require("https");
const http = require("http");

const REPO = "srobinson/attention-matters";
const BIN_NAME = "am";

const PLATFORM_MAP = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "linux-x64": "x86_64-unknown-linux-gnu",
};

function getPlatformKey() {
  const platform = process.platform;
  const arch = process.arch;
  return `${platform}-${arch}`;
}

function getTarget() {
  const key = getPlatformKey();
  const target = PLATFORM_MAP[key];
  if (!target) {
    console.error(
      `Unsupported platform: ${key}\n` +
        `Supported: ${Object.keys(PLATFORM_MAP).join(", ")}`
    );
    process.exit(1);
  }
  return target;
}

function getVersion() {
  const pkg = JSON.parse(
    fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8")
  );
  return pkg.version;
}

function fetch(url) {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith("https") ? https : http;
    mod
      .get(url, { headers: { "User-Agent": "attention-matters-installer" } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return fetch(res.headers.location).then(resolve, reject);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        }
        const chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

async function install() {
  const target = getTarget();
  const version = getVersion();
  const artifact = `am-${target}.tar.gz`;
  const url = `https://github.com/${REPO}/releases/download/v${version}/${artifact}`;

  const binDir = path.join(__dirname, "..", "bin");
  fs.mkdirSync(binDir, { recursive: true });

  const binPath = path.join(binDir, BIN_NAME);

  // Skip download if binary already exists (e.g. CI caching)
  if (fs.existsSync(binPath)) {
    return;
  }

  console.log(`Downloading ${BIN_NAME} v${version} for ${target}...`);

  try {
    const tarball = await fetch(url);

    // Write tarball to temp file, extract with tar
    const tmpTar = path.join(binDir, `${BIN_NAME}.tar.gz`);
    fs.writeFileSync(tmpTar, tarball);
    execSync(`tar xzf "${tmpTar}" -C "${binDir}"`, { stdio: "pipe" });
    fs.unlinkSync(tmpTar);

    // Ensure the binary is executable
    fs.chmodSync(binPath, 0o755);

    console.log(`Installed ${BIN_NAME} v${version} to ${binPath}`);
  } catch (err) {
    console.error(
      `Failed to download ${BIN_NAME} v${version} for ${target}:\n` +
        `  ${err.message}\n\n` +
        `You can install manually:\n` +
        `  cargo install am-cli\n` +
        `  brew install srobinson/tap/am`
    );
    // Don't fail the install — the bin wrapper will show a helpful error
  }
}

install();
