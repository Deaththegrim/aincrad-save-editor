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
> to a separate optional tool) lives in its own dedicated repo,
> [aincrad-save-editor-noscan](https://github.com/Deaththegrim/aincrad-save-editor-noscan),
> for sites (and antivirus) that reject the integrated one. **If your AV
> quarantined this build, grab that one instead** — same editor, no scanner.

## Changes

**0.1.9** — the **Hair** tab now has an *NPC hairstyles* section (shown when the
`hairswap` UE4SS mod is installed). NPC-only hairstyles can't be written to a save —
the game resolves them natively and would crash — so the editor writes your pick to
the mod's config and it applies the style to your character in-game. Requires the
mod loader / UE4SS + the `hairswap` mod.

**0.1.8** — adds a **Match face to skin tone** button that repairs a face whose tone
drifted from the body skin colour.

**0.1.7** — **locks out the body `MeshScale` slider** (editing it resized every
character and mob in the game) and adds a **Fix character scale** button to repair a
save that already hit the bug.

**0.1.6** — editing your **skin tone** now re-tints the face so the two stay matched.

**0.1.5** — fixes a save-corruption bug where **changing your character name** could
fail with `save length … is not a multiple of 16` and refuse to save. Renaming now
works with any name (accented, Japanese, emoji, spaces, or empty). Both builds now
share one codebase: the no-scan build is just `--no-default-features` (the `keyscan`
feature off), so it ships with **zero** `OpenProcess`/`ReadProcessMemory` imports.

## Download

Grab the latest Windows build from [Releases](https://github.com/Deaththegrim/aincrad-save-editor/releases).
Unzip anywhere and run `aml-save-editor.exe` — it's portable (everything it writes
stays in `aml-data/` next to the exe; nothing touches system folders).

## Using it

1. Launch it. On first run it asks for your Echoes of Aincrad pak AES key.
2. No key? Launch the game, get into the world, then click **Recover from running
   game** — it reads your key from your own game and stores it locally.
   (On Linux/SteamOS run the editor in Desktop Mode; recovery reads the running
   game via `/proc`.)
3. Edit appearance; **Apply to game** writes back (timestamped backup first).

Already have a key stored and want to change it or re-recover? Click **Change key**
in the top bar to return to the key screen at any time.

It never ships or uploads a key, and it edits a copy of your save — your live save
is only touched via *Apply to game*.

## Build from source

```
cargo build --release -p aml-save-editor
# Windows, cross-compiled from Linux with the MSVC toolchain (cargo-xwin):
cargo install cargo-xwin && rustup target add x86_64-pc-windows-msvc
cargo xwin build --release -p aml-save-editor --target x86_64-pc-windows-msvc
```

### Packaging the release zips

`scripts/package.sh` builds both portable zips (Linux native + Windows MSVC) into
`dist/`. It reuses the `aml-data/` payload (thumbnails + CJK font, generated from the
game) from the existing dist zips and swaps in a fresh binary, and it **verifies each
binary's embedded version matches `Cargo.toml`** — manual packaging has shipped a
mislabeled build before.

### Publishing to Nexus

`scripts/nexus-upload.sh` pushes a built zip to Nexus via the **v3 upload API**
(create session → PUT to presigned URL → finalise → poll → create file version).
It **dry-runs by default**; add `--publish` and a key (`--key` / `NEXUS_API_KEY`,
from nexusmods.com/settings/api-keys) to actually upload:

```
scripts/nexus-upload.sh --file dist/aml-save-editor-windows-x86_64.zip \
  --file-id <FILE_ID> --version 0.1.9 --name "Aincrad Save Editor 0.1.9" --publish
```

`--file-id` adds a new version to an existing mod file; `--mod-id <game-scoped-id>`
creates a brand-new file. Nexus also ships an official
[GitHub Action](https://github.com/marketplace/actions/upload-to-nexus-mods) that
wraps the same v3 flow for CI.

Unsigned build — Windows SmartScreen may warn on first run. Source is right here.
MIT licensed.
