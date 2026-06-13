# Cairn landing page — design

**Date:** 2026-06-14
**Status:** Approved direction; ready for implementation plan.
**Scope:** A single, self-contained marketing/front-door page for `cairn-ehr.org`, hosted on
Cloudflare Pages. The existing MkDocs spec site moves to `docs.cairn-ehr.org`. This spec covers
*only* the landing page and its assets — not the docs-subdomain DNS/migration work (noted as a
follow-up).

---

## 1. Goal & audience

One page that, in order of visual hierarchy, **makes the case → presents the mission → recruits
contributors → routes serious readers into the specification.**

Audiences, in priority order:
1. **Clinicians and health-IT engineers** who might contribute or adopt.
2. **Health systems / potential allies / funders** evaluating whether Cairn is real and credible.
3. **Curious passers-by** who should leave understanding *why* Cairn exists.

**Primary call to action:** "Read the specification" → `https://docs.cairn-ehr.org`.
**Secondary call to action:** "View on GitHub" → the project repository.
There is deliberately no "install" / "download" CTA — the project is honest that it is in the
architecture & specification phase.

This page is itself held to the project's voice: honest, calm, anti-corporate, clinically
grounded. No growth-hack patterns, no fake urgency, no confirmation-dialog-style dark patterns.

---

## 2. Build & deployment approach

**Self-contained static page. No framework, no build step.**

- `web/index.html` — semantic HTML for the whole page.
- `web/styles.css` — one stylesheet; CSS custom properties for the palette and type scale.
- `web/assets/` — logo (SVG + PNG fallback), favicon, self-hosted fonts (woff2), social card.
- No JavaScript framework. At most a few lines of vanilla JS for the mobile nav toggle and
  smooth-scroll; the page must be fully usable with JS disabled.
- **Cloudflare Pages**, configured with **no build command** and **output directory `web/`**.
  Static files served directly. Free tier, no Worker required for v1.

**Why this approach:** it is the most mission-coherent option — fully inspectable, zero
proprietary dependency, no mandatory cloud build, and it loads instantly even on the intermittent,
low-bandwidth connections Cairn is explicitly designed to serve. A static-site generator or a
CDN-loaded CSS framework would add tooling or an external dependency for what is, today, one page.
If the site later grows to multiple pages, revisit (a generator becomes worth it then).

**Directory note:** files live in `web/` because `site/` is reserved (and gitignored) for the
MkDocs build output.

---

## 3. Visual identity

**Direction: warm paper + stone** — matching the logo, reading durable, humane, and
anti-corporate.

### Palette (CSS custom properties)

| Token | Value | Role |
|---|---|---|
| `--paper` | `#F4F1E9` | page background (warm off-white) |
| `--paper-alt` | `#EFEBE1` | alternate section background (recruit band) |
| `--ink` | `#262A2C` | primary text |
| `--ink-soft` | `#4A5358` | secondary text / nav |
| `--ink-faint` | `#7A827F` | captions, status line, footer |
| `--navy` | `#1E3A52` | bottom stone; "why this exists" band; primary button |
| `--navy-soft` | `#DCE4EA` | text on the navy band |
| `--teal` | `#2F6E62` | top stone; mission icons; recruit-CTA button; links |
| `--teal-light` | `#BFE0D5` | emphasis text on navy band; etched-line accents |
| `--stone` | `#8B9398` | middle stone; neutral dividers |
| `--hairline` | `rgba(30,58,82,0.12)` | section borders / card outlines |

Exact hexes may be nudged ±a few values during build to match the logo PNG precisely; the
relationships above are what matters. Flat fills only — no gradients, consistent with the logo.

**Contrast:** all text/background pairs must meet WCAG AA (4.5:1 body, 3:1 large). The navy band
and the on-paper body text are the pairs to verify explicitly.

### Logo & assets

- Re-create the **stone mark** as a crisp **SVG** (three stacked stones — teal top, stone-grey
  middle, navy bottom — with the subtle etched trend/EKG lines from the logo). Used in the header
  lockup and, larger, in the hero. Keep the source PNG `assets/logo_abb1.png` as a fallback and as
  the reference for color/shape.
- Generate a **favicon** (the stone mark) and a **social-share card** (Open Graph / Twitter image,
  1200×630, tagline on paper).
- Optimize: no multi-MB PNGs shipped to the browser; SVG for the mark, compressed PNG only where a
  raster is genuinely needed.

### Typography

Self-hosted **woff2** in `web/assets/fonts/` — **no Google Fonts CDN** (privacy + dependency-free,
consistent with the mission). All AGPL-compatible / open-licensed (e.g. SIL OFL) faces.

- **Sans (UI, headlines, labels):** a sturdy, slightly wide grotesk/humanist sans that echoes the
  geometric `CAIRN` wordmark. Candidates: Inter, Source Sans 3, IBM Plex Sans. Final pick at build.
- **Serif (manifesto prose accent):** a humanist serif for the "why this exists" line and other
  editorial moments, to make the case read like a considered statement rather than UI copy.
  Candidates: Source Serif 4, Newsreader, IBM Plex Serif.
- Sentence case for prose. The wordmark may retain its tracked-caps `CAIRN` treatment as a brand
  element, but body/headings are sentence case.

---

## 4. Page structure (top to bottom)

Single column, max content width ~1080px, generous vertical rhythm. Each numbered block is a
`<section>` with a clear landmark role.

1. **Header / nav** (sticky, paper background, hairline bottom border)
   - Left: stone mark + `CAIRN` wordmark.
   - Right: text links — *Mission*, *Architecture*, *Principles*, *Docs* — and one outlined
     **GitHub** affordance (the only bordered element in the nav).
   - Collapses to a simple menu on narrow viewports.

2. **Hero** (centered, on paper)
   - Stone mark, enlarged.
   - Headline: **"The grid goes down. The chart stays up."**
   - Sub-headline (one sentence): offline-first, vendor-independent EHR; keeps working through any
     outage, runs anywhere from a Raspberry Pi to a hospital cluster, belongs to no vendor.
   - Buttons: **Read the specification** (primary, navy, → docs subdomain) and **View on GitHub**
     (secondary, outlined).
   - Honest status line: *Architecture & specification phase · AGPL-3.0 · PostgreSQL ≥ 18*.

3. **"Why this exists"** (full-bleed **navy** band — the one strong color moment)
   - Small eyebrow label.
   - One idea, in the serif: *no vendor in the room → the one thing that drives every decision is
     what actually happens at the point of care, including at 3 a.m. when the network is down.*
   - Pulls directly from the README's "Why this exists" voice.

4. **The mission** (on paper) — the four promises as a 2×2 card grid (1-col on mobile):
   - Keeps working through any outage.
   - Runs anywhere, for anyone.
   - Belongs to no one but its users.
   - Respects the clinician's time and judgment.
   - Each card: teal outline icon, short title, one-sentence body.

5. **Founding principles** (on paper, condensed) — a compact presentation of the load-bearing
   commitments (availability over consistency; paper-parity; append-only; identity is a claim;
   fractal topology; vendor independence; auditable safety-critical core). Presented as a tight
   numbered list or small grid, each one line — not the full spec prose. Links to the principles
   docs for depth.

6. **Design at a glance** (on paper) — the architecture made scannable. Use the README's
   "Design at a glance" table content (Resilience, Synchronization, Identity, Topology, Foundation,
   Interoperability, Licensing), rendered as a clean two-column table or definition list. This is
   the credibility section for the health-IT reader.

7. **The name** (short editorial moment, on `--paper-alt`) — the cairn metaphor: a hand-built stack
   of stones marking the safe path, needing no power or network; built by accretion, raised by many
   hands, found in nearly every culture. Optionally fold in the backronym. Sets up the recruit ask.

8. **Contribute / get involved** (recruit band) — "Built by accretion. Raised by many hands." +
   the README's invitation that a well-described failure mode from the front line is a genuine
   contribution. **Get involved** button → GitHub (and/or a contact/mailing mechanism if one exists
   — otherwise GitHub issues/discussions).

9. **Footer** — © Cairn · AGPL-3.0; "The name is stewarded for the mission" linking to
   `STEWARDSHIP-OF-THE-NAME`; links to spec, GitHub, principles. Quiet, single hairline top border.

**Source of truth for copy:** the root `README.md` and `docs/spec/index.md`. The page paraphrases
and condenses them; it must not contradict them. If copy and the canonical docs ever diverge, the
docs win and the page is corrected.

---

## 5. Responsiveness & accessibility

- **Mobile-first**, fluid down to ~320px. The 2×2 mission grid and the glance table both collapse
  to single column. Hero type scales with `clamp()`.
- **Semantic HTML**: one `<h1>` (the tagline), logical heading order, `<nav>`, `<main>`,
  `<section>` with `aria-labelledby`, `<footer>`. Skip-to-content link.
- **Keyboard**: all interactive elements focusable with visible focus rings; nav usable without a
  pointer.
- **Contrast**: WCAG AA verified for every text/background pair (see palette note).
- **Reduced motion**: any scroll/transition respects `prefers-reduced-motion`.
- **No-JS**: full content and navigation work with JavaScript disabled.
- **Performance budget**: target < 200 KB total transfer on first load (excluding optional social
  card); fonts subset where practical; no render-blocking third-party requests.

---

## 6. SEO & sharing

- `<title>` and meta description from the site description in `mkdocs.yml`.
- Open Graph + Twitter card tags pointing at the generated social image.
- Canonical URL `https://cairn-ehr.org/`.
- `robots.txt` allowing indexing; a minimal `sitemap.xml` (single URL acceptable for v1).
- Reasonable `lang="en"`, viewport, and theme-color meta.

---

## 7. Out of scope (v1)

- The `docs.cairn-ehr.org` DNS change and MkDocs hosting migration (separate follow-up; the page
  links to the docs subdomain, which can be pointed once ready — links can temporarily target the
  current docs URL or GitHub).
- Any analytics/telemetry. If ever added, must be privacy-respecting and self-hosted or
  cookieless — but default is **none**, consistent with the mission.
- Multi-page marketing site, blog, or CMS.
- A contact form requiring a backend (use GitHub / a mailto for v1).

---

## 8. Acceptance criteria

- [ ] `web/index.html` + `web/styles.css` render the full page structure in §4 with the §3 identity.
- [ ] Stone mark exists as crisp SVG; favicon and social card generated; no multi-MB raster shipped.
- [ ] Fonts self-hosted; no third-party CDN requests at runtime.
- [ ] Primary CTA → `docs.cairn-ehr.org`; secondary → GitHub repo; no install/download CTA.
- [ ] All copy is consistent with `README.md` / `docs/spec/index.md` (no contradictions).
- [ ] Responsive 320px → desktop; mission grid and glance table collapse correctly.
- [ ] WCAG AA contrast verified; keyboard-navigable; works with JS disabled; honors reduced-motion.
- [ ] Total first-load transfer within the performance budget.
- [ ] Deploys to Cloudflare Pages from `web/` with no build command (documented in a short README).
