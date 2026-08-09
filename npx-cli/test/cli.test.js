"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const { platformInfo } = require("../bin/cli.js");

test("release archive names match every published platform family", () => {
  assert.deepEqual(platformInfo("darwin", "x64"), {
    artifact: "smith-x86_64-macos",
    archiveName: "smith-x86_64-macos.tar.gz",
  });
  assert.deepEqual(platformInfo("linux", "arm64", "gnu"), {
    artifact: "smith-aarch64-linux",
    archiveName: "smith-aarch64-linux.tar.gz",
  });
});

test("musl Linux fails before requesting an archive the release does not build", () => {
  assert.throws(
    () => platformInfo("linux", "x64", "musl"),
    /currently require glibc/
  );
});
