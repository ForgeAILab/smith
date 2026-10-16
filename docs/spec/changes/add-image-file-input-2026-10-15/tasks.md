---
created_at: 2026-10-16T02:27:02Z
updated_at: 2026-10-16T02:27:02Z
completed_at:
---

## 1. Read
- [ ] 1.1 Detect PNG, JPEG, GIF and WebP by signature with pixel size
      (`image_header.rs`), and add a bounded image read beside the text read
      in `support.rs` (3.5 MiB).
- [ ] 1.2 `read` returns an image part and a one-line description for image
      files. Other binary files keep the existing refusal; an oversized image
      names its size and the bound.

## 2. Attachments
- [ ] 2.1 TUI `@FILE` of an image attaches an image part through the same
      bounded read, shown like a clipboard attachment.
- [ ] 2.2 `smith -p --image <PATH>` (repeatable) adds image parts to the
      prompt; `--image` with a `cli/*` profile fails before the run starts.

## 3. Tests and docs
- [ ] 3.1 Tests:
      - `read` on each format, an oversized image, a fake `.png`, and a text
        file (unchanged);
      - `@FILE` of an image;
      - `--image`, including a `cli/*` refusal and a missing file;
      - a text-only model sees the placeholder.
- [ ] 3.2 Docs: `docs/headless-protocol.md` (`--image`), `DESIGN.md`, CLI help.
- [ ] 3.3 Gate, and a live check with the `visual` profile labeling a real
      image through `smith -p --image` and through `read`.
