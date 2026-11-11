"""Rebuilds FastHarnessIcons.ttf, the GUI icon font: a subset of Symbols Nerd Font.

Usage, from the repo root:
  uv run --with fonttools python crates/nh-gui/fonts/subset.py <SymbolsNerdFont-Regular.ttf>

The input is SymbolsNerdFont-Regular.ttf from NerdFontsSymbolsOnly.zip of Nerd Fonts
v3.5.1 (https://github.com/ryanoasis/nerd-fonts/releases/tag/v3.5.1). The glyphs are
the code points in glyphs.json next to this script: to add one, copy its name and code
point from the release's glyphnames.json into glyphs.json, run this script, then name
it in ui/icons.slint.
"""

import json
import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

HERE = Path(__file__).resolve().parent
FAMILY = "Fast Harness Icons"
NAMES = {1: FAMILY, 4: FAMILY + " Regular", 6: "FastHarnessIcons-Regular", 16: FAMILY}


def rename(font: TTFont) -> None:
    table = font["name"]
    for name_id, value in NAMES.items():
        for record in [r for r in table.names if r.nameID == name_id]:
            table.setName(value, name_id, record.platformID, record.platEncID, record.langID)


def main(source: str) -> None:
    glyphs = json.loads((HERE / "glyphs.json").read_text(encoding="utf-8"))
    unicodes = sorted({int(code, 16) for code in glyphs.values()})
    font = TTFont(source)
    subsetter = subset.Subsetter(subset.Options())
    subsetter.populate(unicodes=unicodes)
    subsetter.subset(font)
    rename(font)
    out = HERE / "FastHarnessIcons.ttf"
    font.save(out)
    cmap = TTFont(out).getBestCmap()
    missing = [f"{c:x}" for c in unicodes if c not in cmap]
    print(f"{out.name}: {len(unicodes)} code points, {out.stat().st_size} bytes, missing {missing}")


if __name__ == "__main__":
    main(sys.argv[1])
