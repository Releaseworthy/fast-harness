# Fast Harness Icons

`FastHarnessIcons.ttf` is the GUI's icon font: 60 glyphs cut from `SymbolsNerdFont-Regular.ttf` of [Nerd Fonts v3.5.1](https://github.com/ryanoasis/nerd-fonts/releases/tag/v3.5.1) (`NerdFontsSymbolsOnly.zip`, released 2026-08-21) and renamed "Fast Harness Icons" so it never clashes with an installed Nerd Font. The UI names each code point in `ui/icons.slint`.

## Rebuild or add a glyph

1. Download `NerdFontsSymbolsOnly.zip` and `glyphnames.json` of the same release.
2. Copy the glyph's name and code point from `glyphnames.json` into `glyphs.json`.
3. From the repo root: `uv run --with fonttools python crates/fast-harness-gui/fonts/subset.py <path to SymbolsNerdFont-Regular.ttf>`. It prints the count, the size and any missing code point.
4. Name the glyph in `ui/icons.slint` and check it in the shots test.

## Licences

Nerd Fonts' patched and symbol fonts are under the SIL Open Font License 1.1 (`LICENSE-NerdFonts.txt`, from the release). The glyphs come from three sets; their licences, per the Nerd Fonts [license audit](https://github.com/ryanoasis/nerd-fonts/blob/v3.5.1/license-audit.md) (read 2026-11-12):

| Prefix | Project | Licence |
|---|---|---|
| `cod-` | Codicons (Microsoft) | CC BY 4.0 |
| `md-` | Material Design Icons | Apache 2.0 |
| `oct-` | Octicons (GitHub) | MIT |
