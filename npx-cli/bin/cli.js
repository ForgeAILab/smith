#!/usr/bin/env node
"use strict";

const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");
const process = require("node:process");

const PACKAGE = require("../package.json");

const REPO = "ForgeAILab/smith";
const CACHE_ROOT = path.join(os.homedir(), ".smith", "npx");
const USER_AGENT = `${PACKAGE.name}/${PACKAGE.version}`;

function usage() {
  const version = PACKAGE.version;
  console.log(`Smith npm bootstrapper ${version}

Usage:
  npx ${PACKAGE.name} [smith-options]

Examples:
  npx ${PACKAGE.name}                           # interactive TUI
  npx ${PACKAGE.name} -p "explain this repo"    # one headless turn
  npx ${PACKAGE.name} setup                     # guided provider/model setup
  npx ${PACKAGE.name} --install                 # install 'smith' into ~/.local/bin
  npx ${PACKAGE.name} --uninstall               # remove that installed binary

Options handled by the bootstrapper:
  --install                 Copy the smith binary onto your PATH, then exit
  --uninstall               Remove a smith binary this bootstrapper installed
  --install-dir <dir>       Install/uninstall location (default: ~/.local/bin)
  --release <tag|latest>    Download a specific GitHub release tag
  --version                 Show the npm bootstrapper version
  --help                    Show this help

--install never needs sudo: it writes to a user-owned directory.
All other options are passed through to the smith binary.`);
}

function isHelp(args) {
  return args.includes("--help") || args.includes("-h");
}

function isVersion(args) {
  return args.includes("--version") || args.includes("-V");
}

function defaultInstallDir(env = process.env, homedir = os.homedir()) {
  return env.SMITH_INSTALL_DIR || path.join(homedir, ".local", "bin");
}

function parseArgs(argv, packageVersion = PACKAGE.version, env = process.env) {
  const args = [...argv];
  let release =
    env.SMITH_NPX_TAG ||
    (packageVersion === "0.0.0" ? "latest" : `v${packageVersion}`);
  let installDir = defaultInstallDir(env);
  let mode = "run";
  const passthrough = [];

  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--release") {
      const value = args[i + 1];
      if (!value) {
        throw new Error("--release requires a tag value");
      }
      release = value;
      i += 1;
      continue;
    }
    if (arg.startsWith("--release=")) {
      release = arg.slice("--release=".length);
      continue;
    }
    if (arg === "--install-dir") {
      const value = args[i + 1];
      if (!value) {
        throw new Error("--install-dir requires a directory value");
      }
      installDir = value;
      i += 1;
      continue;
    }
    if (arg.startsWith("--install-dir=")) {
      installDir = arg.slice("--install-dir=".length);
      continue;
    }
    if (arg === "--install") {
      mode = "install";
      continue;
    }
    if (arg === "--uninstall") {
      mode = "uninstall";
      continue;
    }
    passthrough.push(arg);
  }

  return { passthrough, release, mode, installDir };
}

function platformInfo(platform = process.platform, archInput = process.arch) {
  if (platform !== "darwin" && platform !== "linux") {
    throw new Error(
      `Unsupported platform ${platform}. Smith release archives currently support macOS and Linux.`
    );
  }

  let osName = platform === "darwin" ? "macos" : "linux";
  let arch = archInput;
  if (arch === "x64") arch = "x86_64";
  if (arch === "arm64") arch = "aarch64";

  if (arch !== "x86_64" && arch !== "aarch64") {
    throw new Error(`Unsupported architecture ${archInput}`);
  }

  const artifact = `smith-${arch}-${osName}`;
  return { artifact, archiveName: `${artifact}.tar.gz` };
}

function request(url, redirects = 0) {
  return new Promise((resolve, reject) => {
    const req = https.get(
      url,
      {
        headers: {
          "user-agent": USER_AGENT,
          accept: "application/vnd.github+json, application/octet-stream, */*",
        },
      },
      (res) => {
        const location = res.headers.location;
        if (
          location &&
          [301, 302, 303, 307, 308].includes(res.statusCode || 0)
        ) {
          res.resume();
          if (redirects > 5) {
            reject(new Error(`Too many redirects for ${url}`));
            return;
          }
          request(new URL(location, url).toString(), redirects + 1)
            .then(resolve)
            .catch(reject);
          return;
        }

        if ((res.statusCode || 0) < 200 || (res.statusCode || 0) >= 300) {
          res.resume();
          reject(new Error(`HTTP ${res.statusCode} fetching ${url}`));
          return;
        }

        resolve(res);
      }
    );
    req.on("error", reject);
  });
}

async function fetchText(url) {
  const res = await request(url);
  return new Promise((resolve, reject) => {
    let body = "";
    res.setEncoding("utf8");
    res.on("data", (chunk) => {
      body += chunk;
    });
    res.on("end", () => resolve(body));
    res.on("error", reject);
  });
}

async function resolveReleaseTag(release) {
  if (release !== "latest") {
    return release;
  }

  const body = await fetchText(`https://api.github.com/repos/${REPO}/releases/latest`);
  const json = JSON.parse(body);
  if (!json.tag_name) {
    throw new Error("GitHub latest release response did not include tag_name");
  }
  return json.tag_name;
}

function parseChecksum(sums, archiveName) {
  for (const line of sums.split(/\r?\n/)) {
    const match = line.match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/);
    if (match && match[2].trim() === archiveName) {
      return match[1].toLowerCase();
    }
  }
  return null;
}

function downloadFile(url, dest, expectedSha256) {
  const temp = `${dest}.tmp-${process.pid}`;
  fs.mkdirSync(path.dirname(dest), { recursive: true });

  return new Promise((resolve, reject) => {
    request(url)
      .then((res) => {
        const total = Number(res.headers["content-length"] || 0);
        let downloaded = 0;
        const hash = crypto.createHash("sha256");
        const file = fs.createWriteStream(temp);

        const cleanup = (error) => {
          file.destroy();
          try {
            fs.unlinkSync(temp);
          } catch {}
          reject(error);
        };

        res.on("data", (chunk) => {
          downloaded += chunk.length;
          hash.update(chunk);
          if (total > 0) {
            const pct = Math.round((downloaded / total) * 100);
            process.stderr.write(`\rDownloading Smith release: ${pct}%`);
          }
        });

        res.on("error", cleanup);
        file.on("error", cleanup);
        file.on("finish", () => {
          const actual = hash.digest("hex");
          if (expectedSha256 && actual !== expectedSha256) {
            cleanup(
              new Error(
                `Checksum mismatch for ${path.basename(dest)}: expected ${expectedSha256}, got ${actual}`
              )
            );
            return;
          }
          fs.renameSync(temp, dest);
          if (total > 0) {
            process.stderr.write("\n");
          }
          resolve();
        });

        res.pipe(file);
      })
      .catch((error) => {
        try {
          fs.unlinkSync(temp);
        } catch {}
        reject(error);
      });
  });
}

function runTar(archive, dest) {
  fs.mkdirSync(dest, { recursive: true });
  childProcess.execFileSync("tar", ["-xzf", archive, "-C", dest], {
    stdio: "pipe",
  });
}

async function ensureRelease(release) {
  const { artifact, archiveName } = platformInfo();
  const tag = await resolveReleaseTag(release);
  const installDir = path.join(CACHE_ROOT, "releases", tag, artifact);
  const readyFile = path.join(installDir, ".ready");
  const binaryPath = path.join(installDir, "smith");

  if (fs.existsSync(readyFile) && fs.existsSync(binaryPath)) {
    return { binaryPath, installDir, tag };
  }

  const archive = path.join(CACHE_ROOT, "archives", tag, archiveName);
  const releaseBase = `https://github.com/${REPO}/releases/download/${tag}`;

  let expectedSha256 = null;
  try {
    const sums = await fetchText(`${releaseBase}/SHA256SUMS`);
    expectedSha256 = parseChecksum(sums, archiveName);
  } catch {}

  if (!fs.existsSync(archive)) {
    console.error(`Fetching Smith ${tag} for ${artifact}...`);
    await downloadFile(`${releaseBase}/${archiveName}`, archive, expectedSha256);
  } else if (expectedSha256) {
    const hash = crypto.createHash("sha256");
    hash.update(fs.readFileSync(archive));
    const actual = hash.digest("hex");
    if (actual !== expectedSha256) {
      fs.unlinkSync(archive);
      console.error(`Cached Smith archive checksum changed; refetching ${archiveName}...`);
      await downloadFile(`${releaseBase}/${archiveName}`, archive, expectedSha256);
    }
  }

  const tempDir = `${installDir}.tmp-${process.pid}`;
  fs.rmSync(tempDir, { recursive: true, force: true });
  fs.mkdirSync(tempDir, { recursive: true });
  runTar(archive, tempDir);

  if (!fs.existsSync(path.join(tempDir, "smith"))) {
    throw new Error("Release archive did not include smith");
  }
  fs.chmodSync(path.join(tempDir, "smith"), 0o755);

  fs.rmSync(installDir, { recursive: true, force: true });
  fs.renameSync(tempDir, installDir);
  fs.writeFileSync(readyFile, `${new Date().toISOString()}\n`);

  return { binaryPath, installDir, tag };
}

function onPath(dir, env = process.env) {
  const entries = (env.PATH || "").split(path.delimiter).filter(Boolean);
  return entries.some((entry) => path.resolve(entry) === path.resolve(dir));
}

function installBinary(sourcePath, installDir, tag) {
  const target = path.join(installDir, "smith");
  fs.mkdirSync(installDir, { recursive: true });

  const temp = `${target}.tmp-${process.pid}`;
  fs.copyFileSync(sourcePath, temp);
  fs.chmodSync(temp, 0o755);
  fs.renameSync(temp, target);

  console.log(`Installed Smith ${tag} -> ${target}`);
  if (onPath(installDir)) {
    console.log("Run 'smith --help' to get started.");
  } else {
    console.log(`${installDir} is not on your PATH. Add it:`);
    console.log(`    export PATH="${installDir}:$PATH"`);
  }
  return target;
}

function uninstallBinary(installDir) {
  const target = path.join(installDir, "smith");
  try {
    fs.unlinkSync(target);
    console.log(`Removed ${target}`);
  } catch (error) {
    if (error.code === "ENOENT") {
      console.log(`Nothing to remove: ${target} does not exist`);
      return;
    }
    throw error;
  }
}

function runBinary(binary, args, env) {
  const child = childProcess.spawn(binary, args, {
    env,
    stdio: "inherit",
  });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code || 0);
  });
  child.on("error", (error) => {
    console.error(`Failed to start smith: ${error.message}`);
    process.exit(1);
  });

  process.on("SIGINT", () => child.kill("SIGINT"));
  process.on("SIGTERM", () => child.kill("SIGTERM"));
}

async function main() {
  const rawArgs = process.argv.slice(2);
  if (isHelp(rawArgs)) {
    usage();
    return;
  }
  if (isVersion(rawArgs)) {
    console.log(PACKAGE.version);
    return;
  }

  const options = parseArgs(rawArgs);

  if (options.mode === "uninstall") {
    uninstallBinary(options.installDir);
    return;
  }

  const release = await ensureRelease(options.release);

  if (options.mode === "install") {
    installBinary(release.binaryPath, options.installDir, release.tag);
    return;
  }

  runBinary(release.binaryPath, options.passthrough, { ...process.env });
}

module.exports = { defaultInstallDir, onPath, parseArgs, platformInfo };

if (require.main === module) {
  main().catch((error) => {
    console.error(`smith npm bootstrap failed: ${error.message}`);
    if (process.env.SMITH_NPX_DEBUG && error.stack) {
      console.error(error.stack);
    }
    process.exit(1);
  });
}
