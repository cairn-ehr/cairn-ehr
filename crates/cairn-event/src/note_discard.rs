//! The audited discard of a draft progress note, `clinical.note-discard.asserted`
//! (sign-in slice 3b-2b).
//! Pure: no clock, no randomness, no I/O. The node passes in the database's stamps.
//!
//! Why a discard is an event at all: a draft is autosaved and can be taken over by a
//! colleague, who may have read it and acted on it. If it is then thrown away, "it
//! never existed" would be a quiet falsehood. So the discarder signs a record that the
//! draft existed, what it said and who held it. It is NOT a note: it never enters
//! `clinical_note`, and its twin says so in its first line.
use crate::note::{unescape_note_markup, EscapedNarrative, StatedTime};
use serde_json::{json, Value};

pub const NOTE_DISCARD_EVENT_TYPE: &str = "clinical.note-discard.asserted";
pub const NOTE_DISCARD_SCHEMA_VERSION: &str = "clinical.note-discard/1";

/// Who wrote a discarded part. Under the current rule every named writer IS the
/// discarder; a key-bearing value can be added later without changing these two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardedWriter {
    Discarder,
    NotKnown,
}

impl DiscardedWriter {
    /// The frozen wire spelling.
    pub fn wire(&self) -> &'static str {
        match self {
            DiscardedWriter::Discarder => "discarder",
            DiscardedWriter::NotKnown => "not-known",
        }
    }

    /// Read a stored wire word back. `None` for a word this build does not know (a later value):
    /// the caller must refuse it, never read it as one of these two.
    pub fn from_wire(word: &str) -> Option<Self> {
        [DiscardedWriter::Discarder, DiscardedWriter::NotKnown]
            .into_iter()
            .find(|w| w.wire() == word)
    }
}

/// One part of the discarded draft, in writing order.
#[derive(Debug, Clone)]
pub struct DiscardedPart<'a> {
    pub writer: DiscardedWriter,
    pub narrative: EscapedNarrative,
    pub stated: StatedTime<'a>,
    pub recording_started: &'a str,
    pub recording_ended: &'a str,
}

/// Build the `clinical.note-discard/1` payload. Keys are frozen. `parts` must be
/// non-empty: an empty slice is a caller bug, which the database floor refuses.
pub fn discard_payload(parts: &[DiscardedPart]) -> Value {
    let parts: Vec<Value> = parts
        .iter()
        .map(|p| {
            let stated = match p.stated {
                StatedTime::Stated(s) => json!(s),
                StatedTime::NotStated => Value::Null,
            };
            json!({
                "writer": p.writer.wire(),
                "narrative": p.narrative.as_str(),
                "stated_time": stated,
                "recording_started": p.recording_started,
                "recording_ended": p.recording_ended,
            })
        })
        .collect();
    json!({ "parts": parts })
}

/// The plaintext legibility twin: a fixed first line, then each part (1-based) under a
/// heading, its text unescaped. No time goes in: it would be an unchecked second copy
/// of the envelope's `t_effective`.
pub fn render_discard_twin(parts: &[DiscardedPart]) -> String {
    let mut out = String::from("Draft discarded; it was never entered in the record.");
    for (i, p) in parts.iter().enumerate() {
        let who = match p.writer {
            DiscardedWriter::Discarder => "written by the discarder",
            DiscardedWriter::NotKnown => "writer not known",
        };
        out.push_str(&format!(
            "\n\nPart {}, {}:\n{}",
            i + 1,
            who,
            unescape_note_markup(p.narrative.as_str())
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(
        w: DiscardedWriter,
        typed: &str,
        stated: StatedTime<'static>,
    ) -> DiscardedPart<'static> {
        DiscardedPart {
            writer: w,
            narrative: EscapedNarrative::of_typed(typed),
            stated,
            recording_started: "2026-11-04T09:00:00+10:00",
            recording_ended: "2026-11-04T09:05:00+10:00",
        }
    }

    #[test]
    fn the_payload_has_exactly_the_frozen_keys() {
        let v = discard_payload(&[
            part(DiscardedWriter::Discarder, "a", StatedTime::NotStated),
            part(
                DiscardedWriter::NotKnown,
                "b",
                StatedTime::Stated("2026-11-04T08:00:00+10:00"),
            ),
        ]);
        let top: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(top, ["parts"]);
        let ps = v["parts"].as_array().unwrap();
        for p in ps {
            let mut keys: Vec<&str> = p.as_object().unwrap().keys().map(|k| k.as_str()).collect();
            keys.sort();
            assert_eq!(
                keys,
                [
                    "narrative",
                    "recording_ended",
                    "recording_started",
                    "stated_time",
                    "writer"
                ]
            );
        }
        assert_eq!(ps[0]["writer"], "discarder");
        assert!(ps[0]["stated_time"].is_null());
        assert_eq!(ps[1]["writer"], "not-known");
        assert_eq!(ps[1]["stated_time"], "2026-11-04T08:00:00+10:00");
    }

    /// A reader of a stored discard must read the word back, never default it: a word this build
    /// does not know is `None`, so the caller refuses it (PR #984 review).
    #[test]
    fn the_writer_word_reads_back_and_an_unknown_word_is_none() {
        for w in [DiscardedWriter::Discarder, DiscardedWriter::NotKnown] {
            assert_eq!(DiscardedWriter::from_wire(w.wire()), Some(w));
        }
        for unknown in ["", "Discarder", "colleague"] {
            assert_eq!(DiscardedWriter::from_wire(unknown), None, "{unknown:?}");
        }
    }

    #[test]
    fn the_narrative_is_escaped_once() {
        let v = discard_payload(&[part(
            DiscardedWriter::Discarder,
            "BP <90 [L]",
            StatedTime::NotStated,
        )]);
        assert_eq!(v["parts"][0]["narrative"], "BP \\<90 [L]");
    }

    #[test]
    fn the_twin_is_the_typed_text_under_part_headings() {
        let twin = render_discard_twin(&[
            part(DiscardedWriter::Discarder, "a *b*", StatedTime::NotStated),
            part(DiscardedWriter::NotKnown, "c", StatedTime::NotStated),
        ]);
        assert_eq!(
            twin,
            "Draft discarded; it was never entered in the record.\n\nPart 1, written by the discarder:\na *b*\n\nPart 2, writer not known:\nc"
        );
    }

    #[test]
    fn the_twin_names_no_time() {
        let twin = render_discard_twin(&[part(
            DiscardedWriter::Discarder,
            "x",
            StatedTime::Stated("2026-11-04T08:00:00+10:00"),
        )]);
        assert!(!twin.contains("2026"), "{twin}");
    }
}
