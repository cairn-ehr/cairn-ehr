# Cairn landing page

Static landing page for <https://cairn-ehr.org> — no build step, no framework, no third-party
runtime requests. All fonts are self-hosted.

## Local preview

```sh
uv run python -m http.server 8799 --directory web
# open http://127.0.0.1:8799/
```

## Deploy (Cloudflare Pages)

Connect this repository to a Cloudflare Pages project with:

- **Build command:** *(none)*
- **Build output directory:** `web`
- **Production branch:** `main`
- **Custom domain:** `cairn-ehr.org`

`web/_headers` sets caching and basic security headers (handled automatically by Cloudflare Pages).

The MkDocs specification site is deployed separately at `docs.cairn-ehr.org`.

## Files

| Path | Purpose |
|---|---|
| `index.html` | The whole page |
| `styles.css` | All styling (palette, type scale, layout, responsive, a11y) |
| `assets/cairn-mark.webp` / `.png` | The Cairn stone-stack logo (header + hero); WebP primary, PNG fallback. Background is flood-filled transparent; regenerate from `../../assets/cairn_logo_only_320px.png` if the source art changes |
| `assets/favicon.svg` | Favicon (simple stone glyph, legible at 16px) |
| `assets/og-card.svg` / `.png` | Social-share card (1200×630); regenerate the PNG from the SVG if the card text changes |
| `assets/fonts/*.woff2` | Self-hosted Inter + Source Serif 4 (SIL OFL) |
| `robots.txt`, `sitemap.xml` | Indexing |
| `_headers` | Cloudflare Pages caching + security headers |

### Regenerating the social card

```sh
rsvg-convert -w 1200 -h 630 assets/og-card.svg -o assets/og-card.png
```
