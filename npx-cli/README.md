# Smith npm bootstrapper

Run Smith from npm without installing a platform binary first:

```bash
npx @forgeailab/smith
```

The package downloads the matching Smith GitHub release archive for macOS,
glibc Linux, or musl Linux, verifies it against `SHA256SUMS` when available,
caches it under `~/.smith/npx`, and starts the `smith` binary.

Useful commands:

```bash
npx @forgeailab/smith -p "explain this repo"   # one headless turn
npx @forgeailab/smith setup                     # guided provider/model setup
npx @forgeailab/smith --help
```

By default, the wrapper downloads the GitHub release tag that matches the npm
package version. For testing a different release:

```bash
npx @forgeailab/smith --release latest
SMITH_NPX_TAG=v0.1.0 npx @forgeailab/smith
```
