# Relay WASM module

A mailbox shared by every Smith session of one user, so sessions can message
each other. The plugin needs no network: messages are files in its own
`plugin-data` directory, which all sessions of the same user see.

It contributes four tools (`join`, `peers`, `send`, `inbox`), a context
contributor and watcher that announce waiting mail, and a `relay` status
item with the session's name and unread count.

From the Smith checkout:

```sh
rustup target add wasm32-unknown-unknown --toolchain stable
bash examples/wasm-module-relay/build.sh
smith plugin add ./examples/wasm-module-relay
```

In one session ask Smith to join the relay as `alice`; in another, to join as
`bob` and send `alice` a message. A session that never joins answers to a
generated name such as `s-b187cfe8`.

The host polls the watcher at most once every five seconds and wakes an idle
live root session when new mail arrives. The model reads the messages with
`inbox`; notices do not consume them. The plugin requests `session_notify`
along with plugin-data reads and writes. It cannot wake a stopped process.

See [WASM module authoring and limits](../../docs/wasm-modules.md).
