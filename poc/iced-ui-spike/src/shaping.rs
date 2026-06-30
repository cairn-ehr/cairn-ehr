//! Headless shaping check for Spike 0004 claim **I1** (`--features shaping`).
//!
//! Why a proxy shaper: iced renders text through **cosmic-text**, which performs
//! complex-script shaping via **rustybuzz** (a pure-Rust HarfBuzz port) and font
//! discovery via **fontdb**. We shape the [`crate::corpus`] samples with exactly
//! that rustybuzz+fontdb pair, so the question answered — "does iced's text stack
//! turn Arabic/Devanagari/Han into real, non-tofu glyph clusters?" — is faithful,
//! while staying a small self-contained check that runs in `cargo test` instead of
//! needing the whole GUI on a display.
//!
//! Honest limit: this verifies *shaping* (glyphs exist, no `.notdef`), the
//! mechanical half of claim I1. Visual RTL/bidi correctness (**I2**) and live IME
//! commit (**I3**) still need the operator's eyes on the running GUI — shaping
//! cannot prove caret behaviour. A SKIP here (no font) is a SKIP, never a PASS:
//! the spike must not read "shapes fine" off a machine that has no font to shape with.

use crate::corpus::Sample;

/// Outcome of shaping one sample.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeResult {
    /// Shaped successfully: `clusters` real glyphs produced, none `.notdef`.
    Shaped { clusters: usize },
    /// Glyphs were produced but at least one was `.notdef` (tofu) — a FAIL: the
    /// installed font lacks coverage for this script even though shaping ran.
    Tofu { notdef: usize, total: usize },
    /// No installed font covers this sample's characters — a SKIP, not a verdict.
    /// The operator must install a Noto-class font and re-run.
    SkippedNoFont,
}

impl ShapeResult {
    /// Did this sample clear claim I1's bar (shaped, no tofu, meets the floor)?
    /// `SkippedNoFont` is deliberately *not* a pass — the caller must surface it.
    pub fn passes(&self, min_clusters: usize) -> bool {
        matches!(self, ShapeResult::Shaped { clusters } if *clusters >= min_clusters)
    }
}

/// Find the first installed font face that has a glyph for every character in
/// `text`, returning its raw bytes + face index. This mirrors what cosmic-text's
/// fallback does: pick a face that actually covers the script. Returns `None`
/// when nothing installed covers the text → the caller reports `SkippedNoFont`.
fn first_covering_face(db: &fontdb::Database, text: &str) -> Option<(Vec<u8>, u32)> {
    for face in db.faces() {
        let covers = db
            .with_face_data(face.id, |data, index| {
                ttf_parser::Face::parse(data, index)
                    .map(|f| text.chars().all(|c| f.glyph_index(c).is_some()))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if covers {
            // Re-extract the bytes to own them (with_face_data only lends them).
            let owned = db.with_face_data(face.id, |data, index| (data.to_vec(), index));
            if let Some(pair) = owned {
                return Some(pair);
            }
        }
    }
    None
}

/// Shape one corpus sample with rustybuzz, using a system font that covers it.
///
/// Pure-ish: the only side effect is reading installed fonts via `db`, passed in
/// by the caller so font discovery happens once.
pub fn shape_sample(db: &fontdb::Database, sample: &Sample) -> ShapeResult {
    let Some((font_bytes, index)) = first_covering_face(db, sample.text) else {
        return ShapeResult::SkippedNoFont;
    };

    let Some(face) = rustybuzz::Face::from_slice(&font_bytes, index) else {
        return ShapeResult::SkippedNoFont;
    };

    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(sample.text);
    // Let rustybuzz auto-detect script/direction from the text; this is the same
    // path cosmic-text drives for runs it has segmented.
    let glyphs = rustybuzz::shape(&face, &[], buffer);

    let infos = glyphs.glyph_infos();
    // glyph_id 0 is `.notdef` — the "tofu" box. Any of these means missing coverage.
    let notdef = infos.iter().filter(|g| g.glyph_id == 0).count();
    let total = infos.len();

    if total == 0 {
        return ShapeResult::SkippedNoFont;
    }
    if notdef > 0 {
        return ShapeResult::Tofu { notdef, total };
    }
    ShapeResult::Shaped { clusters: total }
}

/// Load system fonts once for the whole corpus run.
pub fn load_system_fonts() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::corpus;

    #[test]
    fn shape_result_skip_is_not_a_pass() {
        // The load-bearing honesty check: a missing-font SKIP must never read PASS.
        assert!(!ShapeResult::SkippedNoFont.passes(1));
        assert!(!ShapeResult::Tofu { notdef: 1, total: 3 }.passes(1));
        assert!(ShapeResult::Shaped { clusters: 5 }.passes(3));
        assert!(!ShapeResult::Shaped { clusters: 2 }.passes(3)); // below floor
    }

    #[test]
    fn corpus_shapes_or_skips_but_never_tofus_silently() {
        // Integration-ish: run the real shaper over the real corpus against
        // whatever fonts this machine has. We assert the *honest* property the
        // spike depends on: every sample either PASSES (shaped, meets floor) or
        // SKIPS (no font) — a Tofu result is a genuine FAIL we want to see loudly.
        //
        // This is intentionally tolerant of a font-less CI box (all SKIP) while
        // still catching a real coverage regression on a box that *has* fonts.
        let db = load_system_fonts();
        let mut shaped = 0;
        let mut skipped = 0;
        for sample in corpus() {
            match shape_sample(&db, &sample) {
                ShapeResult::Shaped { clusters } => {
                    assert!(
                        clusters >= sample.min_clusters,
                        "{}: shaped {clusters} clusters, floor {}",
                        sample.label,
                        sample.min_clusters
                    );
                    shaped += 1;
                }
                ShapeResult::SkippedNoFont => {
                    eprintln!("SKIP {}: no installed font covers it", sample.label);
                    skipped += 1;
                }
                ShapeResult::Tofu { notdef, total } => {
                    panic!(
                        "FAIL {}: {notdef}/{total} glyphs were tofu (.notdef) — \
                         font present but lacks script coverage",
                        sample.label
                    );
                }
            }
        }
        eprintln!("shaping check: {shaped} shaped, {skipped} skipped (no font)");
        // No hard assertion on the shaped count: a font-less environment is a
        // legitimate SKIP-all, which the operator README tells you how to fix.
    }
}
