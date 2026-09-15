# Smith npm bootstrapper

Run Smith from npm without installing a platform binary first:

```bash
npx @forgeailab/smith
```

The package downloads the matching Smith GitHub release archive for macOS or
Linux, verifies it against `SHA256SUMS` when available, caches it under
`~/.smith/npx`, and starts the `smith` binary. Linux always uses Smith's
fully static musl archive, so the same package works on glibc and musl hosts.
Explicitly named `smith-<arch>-linux-gnu.tar.gz` archives remain available
for users who prefer an Ubuntu 22.04-baseline GNU build.

Useful commands:

```bash
npx @forgeailab/smith -p "explain this repo"   # one headless turn
npx @forgeailab/smith setup                     # guided provider/model setup
npx @forgeailab/smith --help
```

## Installing a permanent `smith` command

`--install` copies the downloaded binary onto your PATH so later runs skip npm
entirely. It writes to a user-owned directory, so it never needs sudo:

```bash
npx @forgeailab/smith --install                      # -> ~/.local/bin/smith
npx @forgeailab/smith --install --install-dir ~/bin  # somewhere else
SMITH_INSTALL_DIR=~/bin npx @forgeailab/smith --install
npx @forgeailab/smith --uninstall                    # remove it again
```

If the install directory is not on your `PATH`, the command prints the
`export PATH=...` line to add to your shell profile.

By default, the wrapper downloads the GitHub release tag that matches the npm
package version. For testing a different release:

```bash
npx @forgeailab/smith --release latest
SMITH_NPX_TAG=v0.1.0 npx @forgeailab/smith
```
