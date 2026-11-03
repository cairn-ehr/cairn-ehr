//! The text progress note, `clinical.note.asserted` (ADR-0041, sign-in slice 3a).
//! Pure: no clock, no randomness, no I/O. The node stamps the recording times and
//! passes them in; this module never parses or reformats them.
use serde_json::{json, Value};

/// Event type of a text progress note.
pub const NOTE_EVENT_TYPE: &str = "clinical.note.asserted";
/// Payload schema tag; additive-only evolution (principle 11).
pub const NOTE_SCHEMA_VERSION: &str = "clinical.note/1";

/// The writer's claim about when the note's content applies (principle 4:
/// "no stated time" is a first-class value, not a fabricated one).
#[derive(Debug, Clone, Copy)]
pub enum StatedTime<'a> {
    /// An RFC 3339 instant the writer asserted (freely backdatable).
    Stated(&'a str),
    /// The writer said nothing; recording start stands in, and the payload says so.
    NotStated,
}

/// One note, ready to seal. `narrative` is the already-ESCAPED text
/// (see [`escape_note_markup`]); drafts hold the raw text, only signing escapes it.
pub struct NoteAssertion<'a> {
    pub narrative: &'a str,
    pub stated: StatedTime<'a>,
    /// RFC 3339 UTC, stamped by the node when typing began.
    pub recording_started: &'a str,
    /// RFC 3339 UTC, stamped by the node at the last keystroke.
    pub recording_ended: &'a str,
}

/// Build the `clinical.note/1` payload. Keys are frozen; `media`, `refs` and
/// `encounter` are reserved slots written empty so adding content later is
/// additive, never a shape change.
pub fn note_payload(a: &NoteAssertion) -> Value {
    let basis = match a.stated {
        StatedTime::Stated(_) => "stated",
        StatedTime::NotStated => "recording-started",
    };
    json!({
        "narrative": a.narrative,
        "media": [],
        "refs": [],
        "encounter": null,
        "recording_started": a.recording_started,
        "recording_ended": a.recording_ended,
        "effective_basis": basis,
    })
}

/// Envelope `t_effective`: always set — the stated time, else when recording began.
pub fn note_t_effective(a: &NoteAssertion) -> String {
    match a.stated {
        StatedTime::Stated(t) => t.to_string(),
        StatedTime::NotStated => a.recording_started.to_string(),
    }
}

/// Legibility twin (ADR-0041 decision 4, no media): the narrative itself.
pub fn render_note_twin(a: &NoteAssertion) -> String {
    a.narrative.to_string()
}

/// Make free text inert as markup: `<` -> `\<`, `(` right after `]` -> `\(`
/// (kills `[x](url)` links), `:` right after `]` -> `\:` (kills reference-style
/// link definitions `[ref]: http://...`, which would smuggle in an external URL
/// that ADR-0041 excludes), and every existing `\` doubled so the escape is
/// reversible on ANY input. Apply exactly once, at signing.
pub fn escape_note_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev = '\0';
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\<"),
            '(' | ':' if prev == ']' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
        prev = c;
    }
    out
}

/// Inverse of [`escape_note_markup`]: drop a backslash that precedes ASCII
/// punctuation, so the window shows what was typed.
pub fn unescape_note_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' && it.peek().is_some_and(|n| n.is_ascii_punctuation()) {
            out.push(it.next().expect("peeked"));
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORPUS: [&str; 10] = [
        "BP <90",
        "<5 min",
        "pt said \"<b>\"",
        "[L] arm (R)",
        "see [x](http://e.org)",
        "ends with \\",
        "a\\<b",
        "plain",
        "[ref]: http://e.org",
        "[sic]: he said",
    ];

    fn note(stated: StatedTime<'static>) -> NoteAssertion<'static> {
        NoteAssertion {
            narrative: "chest clear",
            stated,
            recording_started: "2026-11-03T01:00:00Z",
            recording_ended: "2026-11-03T01:05:00Z",
        }
    }

    /// True if byte `i` is preceded by an even number of backslashes (i.e. not escaped).
    fn unescaped_at(b: &[u8], i: usize) -> bool {
        b[..i].iter().rev().take_while(|&&c| c == b'\\').count() % 2 == 0
    }

    #[test]
    fn payload_has_exactly_the_frozen_keys() {
        let v = note_payload(&note(StatedTime::NotStated));
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        let mut want = vec![
            "narrative",
            "media",
            "refs",
            "encounter",
            "recording_started",
            "recording_ended",
            "effective_basis",
        ];
        want.sort();
        assert_eq!(keys, want);
        assert_eq!(v["media"], json!([]));
        assert_eq!(v["refs"], json!([]));
        assert!(v["encounter"].is_null());
    }

    #[test]
    fn no_stated_time_uses_recording_start_and_says_so() {
        let a = note(StatedTime::NotStated);
        assert_eq!(note_payload(&a)["effective_basis"], "recording-started");
        assert_eq!(note_t_effective(&a), a.recording_started);
    }

    #[test]
    fn a_stated_time_is_kept_verbatim() {
        let a = note(StatedTime::Stated("2026-11-03T03:00:00Z"));
        assert_eq!(note_payload(&a)["effective_basis"], "stated");
        assert_eq!(note_t_effective(&a), "2026-11-03T03:00:00Z");
    }

    #[test]
    fn twin_is_the_narrative() {
        let a = note(StatedTime::NotStated);
        assert_eq!(render_note_twin(&a), a.narrative);
    }

    #[test]
    fn escape_corpus_round_trips() {
        for s in CORPUS {
            assert_eq!(unescape_note_markup(&escape_note_markup(s)), s, "{s:?}");
        }
    }

    #[test]
    fn escaped_text_has_no_unescaped_tag_or_link() {
        for s in CORPUS {
            let e = escape_note_markup(s);
            let b = e.as_bytes();
            for i in 0..b.len() {
                let tag = b[i] == b'<'
                    && b.get(i + 1)
                        .is_some_and(|c| c.is_ascii_alphabetic() || b"/!?".contains(c));
                let link = b[i] == b']' && matches!(b.get(i + 1), Some(b'(') | Some(b':'));
                assert!(
                    !((tag || link) && unescaped_at(b, i)),
                    "live markup at {i} in {e:?} (from {s:?})"
                );
            }
        }
    }

    /// Every string of length 0..=5 over the hostile alphabet: reversible and inert.
    #[test]
    fn exhaustive_small_alphabet_round_trips_and_is_inert() {
        let alphabet = ['\\', '<', '(', ']', ':', 'a'];
        let mut level: Vec<String> = vec![String::new()];
        for _ in 0..=5 {
            for s in &level {
                let e = escape_note_markup(s);
                assert_eq!(unescape_note_markup(&e), *s, "round trip {s:?}");
                let b = e.as_bytes();
                for i in 0..b.len() {
                    let live = b[i] == b'<'
                        || (b[i] == b']' && matches!(b.get(i + 1), Some(b'(') | Some(b':')));
                    assert!(
                        !(live && unescaped_at(b, i)),
                        "live markup in {e:?} from {s:?}"
                    );
                }
            }
            level = level
                .iter()
                .flat_map(|s| alphabet.iter().map(move |c| format!("{s}{c}")))
                .collect();
        }
    }
}
