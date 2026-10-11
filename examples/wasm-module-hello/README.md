# Hello WASM module

Copyable plugin layout: `.smith-plugin/plugin.json` declares `module.wasm` and
one capability (`workspace_read`). The guest contributes one `hello` tool that
reads the current workspace's `README.md`, and one cached `hello-status` item.
The tool still uses ordinary approval, even though its declared effect is read.

From the Smith checkout:

```sh
rustup target add wasm32-unknown-unknown --toolchain stable
bash examples/wasm-module-hello/build.sh
smith plugin add ./examples/wasm-module-hello
```

Ask Smith to call `wasm_17_wasm-module-hello_hello__hello` in a workspace with a
`README.md`. The status changes from `hello ready` to `hello called`. `/modules`
shows the stable configuration id and `/plugins` shows its grant.

`module.wasm` is included for users without a Rust toolchain. To author a copy,
update the plugin name, guest descriptor, and contributions. The local SDK path
in Cargo.toml assumes this repository layout; adjust it to your SDK checkout
when copying elsewhere. `build.sh` uses Smith's fixture componentiser; point that
path at your Smith checkout as well. Both dependencies use exact versions in
Cargo.lock. Build for `wasm32-unknown-unknown`, not a WASI target.

See [WASM module authoring and limits](../../docs/wasm-modules.md).
