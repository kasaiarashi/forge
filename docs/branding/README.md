# Forge VCS branding assets

The canonical source files for the Forge logo, in two variants. **Use the
simple variant in any small space** (favicons, navbar glyph, footer mark,
UE plugin tile, taskbar icon). Use the full wordmark variant anywhere
there's room for the type to read clearly (login screen, README header,
release pages, marketing).

## Files

| File | Use it for |
|---|---|
| `forge-logo.svg` / `.png` | Full wordmark — "FORGE" + glyph. Hero placements: README, login screen, setup wizard, release art. |
| `forge-logo-simple.svg` / `.png` | Glyph only. Small spaces: favicon, navbar, footer copyright row, UE plugin Icon128, Inno Setup icon. |
| `forge.ico` | Multi-size Windows icon (16/24/32/48/64/128/256). Generated from `forge-logo-simple.png`. Used by Inno Setup installers and the `UninstallDisplayIcon` registry entry. |

The PNGs are 2000×2000 black-on-transparent. Resize as needed; the SVGs
scale infinitely and should be preferred wherever the consumer supports
SVG (browsers, Markdown).

## Where the assets are deployed

- **Web UI** — `crates/forge-web/ui/public/forge-logo.svg`,
  `forge-logo-simple.svg`, `forge-logo.png`, `forge-logo-simple.png`,
  `favicon.svg` (= simple variant). The browser uses the favicon link in
  `index.html`; the React components reference the SVGs by their public
  URL (`/forge-logo.svg`, `/forge-logo-simple.svg`).
- **UE plugin** —
  `plugin/ForgeSourceControl/Plugins/ForgeSourceControl/Resources/Icon128.png`
  (128×128, generated from the simple variant). UE looks here by
  convention.
- **Windows installer** — `installers/windows/forge.ico`. Referenced by
  both `forge-server.iss` and `forge-client.iss` via `SetupIconFile=`.

## Regenerating `forge.ico` and `Icon128.png`

If the source SVGs change, rebuild the derived assets with this snippet
(requires Python + Pillow, `pip install --user Pillow`):

```python
from PIL import Image
SRC_SIMPLE = r"docs/branding/forge-logo-simple.png"

# Multi-size .ico for the Windows installer.
Image.open(SRC_SIMPLE).convert("RGBA").save(
    r"installers/windows/forge.ico",
    format="ICO",
    sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)],
)

# 128x128 PNG for the UE plugin Icon128.
im = Image.open(SRC_SIMPLE).convert("RGBA")
im.thumbnail((128, 128), Image.LANCZOS)
canvas = Image.new("RGBA", (128, 128), (255, 255, 255, 0))
canvas.paste(im, ((128 - im.width) // 2, (128 - im.height) // 2), im)
canvas.save(
    r"plugin/ForgeSourceControl/Plugins/ForgeSourceControl/Resources/Icon128.png",
    optimize=True,
)
```
