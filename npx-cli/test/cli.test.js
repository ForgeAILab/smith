"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const os = require("node:os");
const path = require("node:path");

const {
  defaultInstallDir,
  onPath,
  parseArgs,
  platformInfo,
} = require("../bin/cli.js");

test("release archive names match every canonical platform family", () => {
  assert.deepEqual(platformInfo("darwin", "x64"), {
    artifact: "smith-x86_64-macos",
    archiveName: "smith-x86_64-macos.tar.gz",
  });
  assert.deepEqual(platformInfo("darwin", "arm64"), {
    artifact: "smith-aarch64-macos",
    archiveName: "smith-aarch64-macos.tar.gz",
  });
  assert.deepEqual(platformInfo("linux", "x64"), {
    artifact: "smith-x86_64-linux",
    archiveName: "smith-x86_64-linux.tar.gz",
  });
  assert.deepEqual(platformInfo("linux", "arm64"), {
    artifact: "smith-aarch64-linux",
    archiveName: "smith-aarch64-linux.tar.gz",
  });
});

test("unsupported platforms and architectures fail before download", () => {
  assert.throws(() => platformInfo("win32", "x64"), /Unsupported platform/);
  assert.throws(() => platformInfo("linux", "riscv64"), /Unsupported architecture/);
});

const HOME_BIN = path.join(os.homedir(), ".local", "bin");

test("published package versions select their matching immutable tag", () => {
  assert.deepEqual(parseArgs([], "0.0.2", {}), {
    passthrough: [],
    release: "v0.0.2",
    mode: "run",
    installDir: HOME_BIN,
  });
  assert.deepEqual(parseArgs(["-p", "hello"], "0.0.2", {}), {
    passthrough: ["-p", "hello"],
    release: "v0.0.2",
    mode: "run",
    installDir: HOME_BIN,
  });
});

test("explicit release selection overrides package defaults", () => {
  assert.deepEqual(parseArgs(["--release", "v0.0.1", "--help"], "0.0.2", {}), {
    passthrough: ["--help"],
    release: "v0.0.1",
    mode: "run",
    installDir: HOME_BIN,
  });
  assert.equal(parseArgs([], "0.0.2", { SMITH_NPX_TAG: "next" }).release, "next");
});

test("--install targets a user-writable directory and consumes its own flags", () => {
  assert.deepEqual(parseArgs(["--install"], "0.0.2", {}), {
    passthrough: [],
    release: "v0.0.2",
    mode: "install",
    installDir: HOME_BIN,
  });
  assert.deepEqual(parseArgs(["--install", "--install-dir", "/opt/bin"], "0.0.2", {}), {
    passthrough: [],
    release: "v0.0.2",
    mode: "install",
    installDir: "/opt/bin",
  });
  assert.deepEqual(parseArgs(["--install-dir=/opt/bin", "--uninstall"], "0.0.2", {}), {
    passthrough: [],
    release: "v0.0.2",
    mode: "uninstall",
    installDir: "/opt/bin",
  });
  assert.throws(() => parseArgs(["--install-dir"], "0.0.2", {}), /--install-dir requires/);
});

test("SMITH_INSTALL_DIR overrides the default install directory", () => {
  assert.equal(defaultInstallDir({ SMITH_INSTALL_DIR: "/srv/bin" }), "/srv/bin");
  assert.equal(defaultInstallDir({}, "/home/tester"), path.join("/home/tester", ".local", "bin"));
  assert.equal(parseArgs(["--install"], "0.0.2", { SMITH_INSTALL_DIR: "/srv/bin" }).installDir, "/srv/bin");
});

test("PATH membership ignores trailing separators and relative spellings", () => {
  assert.equal(onPath("/home/tester/.local/bin", { PATH: "/usr/bin:/home/tester/.local/bin" }), true);
  assert.equal(onPath("/home/tester/.local/bin/", { PATH: "/usr/bin:/home/tester/.local/bin" }), true);
  assert.equal(onPath("/home/tester/.local/bin", { PATH: "/usr/bin:/usr/local/bin" }), false);
  assert.equal(onPath("/home/tester/.local/bin", {}), false);
});
