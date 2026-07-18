#!/usr/bin/env python3
"""Regenerate the voice-preview payload (aml-data/save-editor/voices/) from the game.

The character creator's per-voice sample lines live in the Wwise event
`Play_VOFX_AvatarCustomize`: a switch container keyed on `Switch_Avatar_Voice`
(values = the save's voice FNames, Player_M .. Player_F_06) with a nested
6-line switch per voice. Media are loose `Media/<lang>/NN/<id>.wem` files in
pakchunk0. This script re-derives the whole map from the paks — no hardcoded
wem ids — and emits `voices/<en|jp>/<Voice>_<n>.ogg` (n = 1..=6).

Prerequisites on PATH / alongside:
  repak            (cargo install repak_cli) — classic .pak reader
  wwiser.pyz       https://github.com/bnnm/wwiser/releases  (bnk -> xml)
  vgmstream-cli    https://github.com/vgmstream/vgmstream/releases (wem -> wav)
  ffmpeg           (wav -> ogg/vorbis)

Usage:
  extract-voices.py --paks "<game>/EchoesofAincrad/Content/Paks" \
                    --aes-key <hex> --out voices/
Then copy voices/ into the bundle payload: aml-data/save-editor/voices/.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile

try:  # the parsed XML is wwiser's own local output, but harden when possible
    import defusedxml.ElementTree as ET
except ImportError:
    import xml.etree.ElementTree as ET

EVENT_BNK = "EchoesofAincrad/Content/WwiseAudio/Event/{lang}/35/Play_VOFX_AvatarCustomize.bnk"
MEDIA = "EchoesofAincrad/Content/WwiseAudio/Media/{lang}/{nn}/{wid}.wem"
LANGS = {"en": "English(US)", "jp": "Japanese(JP)"}
VOICES = [
    "Player_M", "Player_M_02", "Player_M_03", "Player_M_04", "Player_M_05", "Player_M_06",
    "Player_F", "Player_F_02", "Player_F_03", "Player_F_04", "Player_F_05", "Player_F_06",
]


def fnv(name: str) -> int:
    """Wwise id = FNV-1 32-bit over the lowercased name."""
    h = 2166136261
    for c in name.lower().encode():
        h = (h * 16777619) & 0xFFFFFFFF
        h ^= c
    return h


def run(*cmd, **kw):
    subprocess.run(cmd, check=True, **kw)


def parse_bank(xml_path):
    """voice -> [wem ids] (line order = ascending switch-value id, the creator's 1..6)."""
    vh = {fnv(v): v for v in VOICES}
    switches, sounds, target = {}, {}, None
    root = ET.parse(xml_path).getroot()
    if root is None:
        sys.exit(f"empty XML: {xml_path}")

    def val(el):
        return int(el.get("value") or 0)

    def field(obj, name):
        return next((val(f) for f in obj.iter("field") if f.get("name") == name), 0)

    for obj in root.iter("object"):
        n = obj.get("name")
        if n == "CAkActionPlay":
            target = field(obj, "idExt")
        elif n == "CAkSwitchCntr":
            pkgs = []
            for pkg in obj.iter("object"):
                if pkg.get("name") == "CAkSwitchPackage":
                    sid, nodes = None, []
                    for f in pkg.iter("field"):
                        if f.get("name") == "ulSwitchID":
                            sid = val(f)
                        if f.get("name") == "NodeID":
                            nodes.append(val(f))
                    pkgs.append((sid, nodes))
            switches[field(obj, "ulID")] = pkgs
        elif n == "CAkSound":
            sounds[field(obj, "ulID")] = field(obj, "sourceID")
    out = {}
    for sid, nodes in switches[target]:
        voice = vh.get(sid)
        if not voice or not nodes:
            continue
        out[voice] = [
            sounds[kids[0]]
            for _, kids in sorted(switches[nodes[0]])
            if kids and kids[0] in sounds
        ]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--paks", required=True)
    ap.add_argument("--aes-key", required=True)
    ap.add_argument("--out", default="voices")
    ap.add_argument("--wwiser", default="wwiser.pyz")
    args = ap.parse_args()

    pak = os.path.join(args.paks, "pakchunk0-WindowsClient.pak")
    with tempfile.TemporaryDirectory() as tmp:
        # 1. Event banks out of the pak, parsed to XML.
        run("repak", "--aes-key", args.aes_key, "unpack", pak, "-o", tmp,
            "-i", "**/Play_VOFX_AvatarCustomize.bnk")
        maps = {}
        for lang, folder in LANGS.items():
            bnk = os.path.join(tmp, EVENT_BNK.format(lang=folder))
            run(sys.executable, args.wwiser, "-d", "xml", bnk)
            maps[lang] = parse_bank(bnk + ".xml")
            missing = [v for v in VOICES if not maps[lang].get(v)]
            if missing:
                sys.exit(f"{lang}: no lines resolved for {missing}")

        # 2. The referenced wems.
        includes = []
        for lang, folder in LANGS.items():
            for wids in maps[lang].values():
                for wid in wids:
                    includes += ["-i", MEDIA.format(lang=folder, nn=str(wid)[:2], wid=wid)]
        run("repak", "--aes-key", args.aes_key, "unpack", pak, "-o", tmp, *includes)

        # 3. Decode + encode.
        n_done = 0
        for lang, folder in LANGS.items():
            os.makedirs(os.path.join(args.out, lang), exist_ok=True)
            for voice, wids in maps[lang].items():
                for i, wid in enumerate(wids, 1):
                    wem = os.path.join(tmp, MEDIA.format(lang=folder, nn=str(wid)[:2], wid=wid))
                    wav = os.path.join(tmp, "tmp.wav")
                    ogg = os.path.join(args.out, lang, f"{voice}_{i}.ogg")
                    run("vgmstream-cli", "-o", wav, wem, stdout=subprocess.DEVNULL)
                    run("ffmpeg", "-y", "-loglevel", "error", "-i", wav,
                        "-c:a", "libvorbis", "-q:a", "2", ogg)
                    n_done += 1
        print(f"OK: {n_done} clips -> {args.out}/ "
              f"({json.dumps({l: len(m) for l, m in maps.items()})} voices per lang)")


if __name__ == "__main__":
    main()
