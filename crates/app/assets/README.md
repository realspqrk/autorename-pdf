# GUI assets

Bundled into the binary with `include_bytes!` (`src/gui/theme.rs`). Licenses in `fonts/LICENSES/`.

| File | Source | Version | License | Upstream archive sha256 |
|---|---|---|---|---|
| `fonts/Inter-{Regular,Medium,SemiBold}.subset.ttf` | https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip (`extras/ttf/`) | 4.1 | SIL OFL 1.1 | `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e` |
| `fonts/JetBrainsMono-Regular.subset.ttf` | https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip (`fonts/ttf/`) | 2.304 | SIL OFL 1.1 | `6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf` |
| `fonts/lucide.subset.ttf` | https://registry.npmjs.org/lucide-static/-/lucide-static-1.70.0.tgz (`font/lucide.ttf`) | lucide-static 1.70.0 | ISC | `3d1f09df1c468abe512ee6d6596db0dc773fa46c61050e365f0c5fee5120f16a` |

Subset with fonttools `pyftsubset` (`--no-hinting`; egui rasterizes unhinted):

- Text fonts: `U+0020-007E,U+00A0-00FF,U+0100-017F,U+0180-024F,U+0370-03FF,U+0400-04FF,U+2000-206F,U+20AC,U+2190-21FF,U+2026,U+2212,U+2022`, features `kern,liga,calt` (mono: `kern`).
- Lucide: only the codepoints listed in `theme::icon` (regenerate the subset when adding an icon; codepoints from `font/codepoints.json`).

Subset sha256:

```
d13b7bd4adc3aa2f742285dee56ca84407ddc72b19d0c2f65221fdeb64793be0  Inter-Regular.subset.ttf
d89c440f7437e9defee67dd2718e4269ca25a29939ffe1c9f8579137440f8adc  Inter-Medium.subset.ttf
c963c907b17c42bf937af8978211122b49e339af5955959b2466f60c0c16e8ea  Inter-SemiBold.subset.ttf
53d7e70f1ef4d1fff81d83f70e8e36dea6dbb3dbe1a03269de1903ddb3b2dd4b  JetBrainsMono-Regular.subset.ttf
a322ea41386191e37926924a0a2455fa0af12f69005d7da13f672e1f085a1de4  lucide.subset.ttf
```
