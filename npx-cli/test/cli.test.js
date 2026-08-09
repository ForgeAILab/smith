"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const { parseArgs, platformInfo } = require("../bin/cli.js");

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

test("published package versions select their matching immutable tag", () => {
  assert.deepEqual(parseArgs([], "0.0.2", {}), {
    passthrough: [],
    release: "v0.0.2",
  });
  assert.deepEqual(parseArgs(["-p", "hello"], "0.0.2", {}), {
    passthrough: ["-p", "hello"],
    release: "v0.0.2",
  });
});

test("explicit release selection overrides package defaults", () => {
  assert.deepEqual(parseArgs(["--release", "v0.0.1", "--help"], "0.0.2", {}), {
    passthrough: ["--help"],
    release: "v0.0.1",
  });
  assert.equal(parseArgs([], "0.0.2", { SMITH_NPX_TAG: "next" }).release, "next");
});
