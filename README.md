# Aincrad Save Editor (all-in-one)

A friendly visual editor for **Echoes of Aincrad** character appearance — pick a
face / hair / eyes by sight from the real in-game thumbnails, edit safely on a
working copy, and write back only on confirm (with a timestamped backup).

This is the **all-in-one build**: pak-key recovery is built in, so it can read your
key straight from your running game — no separate tool needed.

> **Distributed here on GitHub on purpose.** Because key recovery reads your
> running game's memory (to recover *your own* pak key), antivirus sandboxes flag
> the binary as `T1055 Process Injection` — a false positive, but enough to get it
> auto-quarantined on some mod sites even with a **clean VirusTotal (0 detections)**.
> GitHub doesn't gate on that, so the convenient integrated build lives here.
>
> A split build of the editor that does **no** memory reading (key recovery moved
> to a separate optional tool) lives in the main
> [aincrad-mod-loader](https://github.com/Deaththegrim/aincrad-mod-loader) repo for
> sites that reject the integrated one.

## Download

Grab the latest Windows build from [Releases](https://github.com/Deaththegrim/aincrad-save-editor/releases).
Unzip anywhere and run `aml-save-editor.exe` — it's portable (everything it writes
stays in `aml-data/` next to the exe; nothing touches system folders).

## Using it

1. Launch it. On first run it asks for your Echoes of Aincrad pak AES key.
2. No key? Launch the game, get into the world, then click **Recover from running
   game** — it reads your key from your own game and stores it locally.
3. Edit appearance; **Apply to game** writes back (timestamped backup first).

It never ships or uploads a key, and it edits a copy of your save — your live save
is only touched via *Apply to game*.

## Build from source

```
cargo build --release -p aml-save-editor
# Windows, cross-compiled from Linux with the MSVC toolchain (cargo-xwin):
cargo install cargo-xwin && rustup target add x86_64-pc-windows-msvc
cargo xwin build --release -p aml-save-editor --target x86_64-pc-windows-msvc
```

Unsigned build — Windows SmartScreen may warn on first run. Source is right here.
MIT licensed.
