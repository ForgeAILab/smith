# Let a text-only model see images

Some main models accept text only. Images then reach them as placeholders
such as `[image 3f9a2c1b: image/png 1200×800 not included in this request]`.
Setting a vision model gives the agent a `view_image` tool: it asks that
model one question about one image and gets a text answer back.

## When to raise it

If an image matters to the user's request and you can only see a
placeholder, say so plainly. Then offer to set up a vision model. Do not
guess what the image shows.

## Steps

1. **Find a model that can see.** Run `smith config explain vision` to see
   the current value (`unset`, or `disabled by <source>`). The capability
   hint in your context lists the user's profiles whose model accepts
   images. Otherwise, look in `~/.smith/config.toml` for a profile whose
   model is known to take images.
2. **Ask the user.** Image bytes are sent to the vision model's provider,
   which may differ from the main provider. Name that provider and get the
   user's agreement before changing anything.
3. **Add one line** at the top level of `~/.smith/config.toml`, before any
   `[table]` header:

   ```toml
   vision = "visual"
   ```

   The value is a profile name, or `"<provider>/<model>"` for a model of a
   declared provider. One profile can override it with its own `vision`
   key.
4. **Verify.** Run `smith config explain vision`. Smith rejects a model that
   does not accept images when the config loads, and names the `vision` key.
5. **Apply.** The user rebuilds the session (`/profile`, `/model`, or
   `/resume`) or restarts Smith. After that, `/modules` lists `vision` as
   mounted and `view_image` is available.

## Turning it off

`vision = false` turns vision off for good and stops the reminder. Put it on
one profile to keep images from one provider:

```toml
[profiles.private]
vision = false
```

## Notes

- A main model that accepts images does not need this; `vision` is then
  inactive and images are sent directly.
- `view_image` calls go through approval, because image bytes leave the
  machine.
- Read-only postures keep `view_image` while `vision` is listed in
  `[agent] read_only_network_modules`, which is the default.
