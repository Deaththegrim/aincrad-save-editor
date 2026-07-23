Aincrad Save Editor __VER__ — portable (with key-recovery)
https://github.com/Deaththegrim/aincrad-save-editor

TO START:  run  Aincrad-Save-Editor.exe  (Windows)
           or   Aincrad-Save-Editor      (Linux)

On first launch, paste your Echoes of Aincrad pak AES key — or click
"Recover key from running game": launch the game (get into the world), click it,
and the editor reads your key from your own running game. The key + your saved
"looks" live in aml-data/ right here in this folder — the editor is fully
portable and never touches system folders.

Safety: the editor works on a COPY of your save. It only touches your real save
via "Apply to game", which makes a timestamped backup first (see the Backups
page in the editor to restore one).

Picker thumbnails and voice previews are included in aml-data/ so the visual
pickers work out of the box.

NOTE: the key-recovery reads the running game's memory, which some antivirus
flags as suspicious (it's a false positive — source is public). If your AV
quarantines this build, use the no-keyscan build instead
(github.com/Deaththegrim/aincrad-save-editor-noscan) and grab the key with the
standalone aml-keyscan tool. Unsigned build: SmartScreen may warn on first run.

Changelog: https://github.com/Deaththegrim/aincrad-save-editor/releases
MIT licensed — see LICENSE.txt.
