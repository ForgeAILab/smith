## Context

Each `/model` change rebuilds the runtime and its provider stack. The
provider is wrapped in layers in `factory/provider.rs`: response policy,
reasoning dialect, and credential pool. The runtime projects image parts per
request (window and text-only placeholders) before the provider sees the
request, but it knows nothing about a provider's byte, pixel or format
limits.

Codex re-prepares images when they are inserted into history and keeps an
LRU of 32 entries and 64 MiB keyed by content hash and mode.

## Goals / Non-Goals

- Goals:
  - No request fails because an image in history exceeds the serving
    provider's limits.
  - No repeated decode cost on every request.
- Non-Goals:
  - Rewriting canonical history.
  - Upscaling an image that was shrunk for a stricter provider.
  - Changing the runtime's image window or placeholders.

## Decisions

- **D1 Provider wrapper.** An `ImageFittingProvider` wraps the provider that
  `factory/provider.rs` builds, innermost below the credential pool, so
  every attempt and retry is fitted. It rewrites `ContentPart::Image` data
  URLs in message and tool-result content. Non-data URLs pass through.
- **D2 Cheap check first.** From the data URL's header, read the format,
  dimensions and encoded size. An image within the limits is sent unchanged
  without a full decode.
- **D3 Bounded cache.** An LRU of 32 entries and 64 MiB of prepared output,
  keyed by a SHA-256 of the data URL plus the limits, shared by one
  provider stack. Codex uses the same bounds.
- **D4 Failure becomes text.** An image that cannot be decoded or fitted is
  replaced in that request by `[image omitted: <reason>]`, matching Codex's
  placeholders, so one bad image does not wedge the session.
- **D5 Ingestion stays.** `read`, `@FILE` and `--image` still prepare at
  ingestion, which bounds history and checkpoint size. The wrapper is the
  backstop that makes a provider switch safe.

## Risks / Trade-offs

- **Token accounting.** The runtime estimates image cost from the image in
  history, so a request that sends a smaller re-fitted image is counted
  high. The overcount is conservative.
- **Latency.** The first request after a switch decodes each oversized
  image once. Later requests hit the cache.
