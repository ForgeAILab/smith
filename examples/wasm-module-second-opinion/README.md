# Second opinion WASM module

A tool that reads a bounded recent conversation through `session_history` and
asks the session's current model for a second opinion through `complete`.
It requests the `model` and `session_history` capabilities. No API key,
provider choice, network grant, or credential enters the component.

```sh
bash examples/wasm-module-second-opinion/build.sh
smith plugin add ./examples/wasm-module-second-opinion
```

Ask Smith to use the second-opinion review tool with a specific question.
This calls the same model as the session; use the built-in advisor when you
want a different reviewer model. The example requests at most 1024 output
tokens and includes up to 8 KiB of recent text. Installed agents control
their own generation limits. User restrictions can lower the request limit;
a refusal is returned as an ordinary tool error.

See [WASM module authoring and limits](../../docs/wasm-modules.md).
