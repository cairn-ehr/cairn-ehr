//! #893 (#895 review): every function that locks the node connection says when the window cut
//! its command off. The rule lives at each call site (`ConnectionGuard::explain`, or a
//! `CutWatch` taken from the guard), and a new site that forgets shows a bare "connection closed"
//! again, the text of a node restart too, with every test still green: no behavioural test runs
//! a cut through each of ~30 call sites. So the source of this crate and of `cairn-gui-live` is
//! read instead.
//!
//! Its limit: it holds per FUNCTION. A function that explains one error and forgets another (a
//! failure that comes back inside an `Ok` outcome, say) passes; those are pinned one by one
//! where they are fixed (`each_judged_pair_failure_is_explained`, `a_cut_off_sign_off_names_the_cut`,
//! `a_side_the_window_cut_off_names_the_cut`).
use crate::source_guard::sources;
use std::path::Path;

/// Functions that lock the node connection but word no error a cut could cause, each with why.
const EXEMPT: &[&str] = &[
    // Records what a gesture cost; its error goes to stderr only, never to the screen.
    "record_timing",
];

/// Any of these in a function's body is its errors passed through the cut check: the guard's own
/// `explain`, a `CutWatch` taken from it, its `cut_off()` question, or `link`'s `as_text` (which
/// explains).
const EXPLAINS: &[&str] = &["explain(", "cut_watch()", "cut_off()", "as_text("];

/// Each function in `src` as (name, code with all whitespace and comment lines removed), from
/// its declaration to the next one. A file's `#[cfg(test)] mod tests` block is left out.
fn functions(src: &str) -> Vec<(String, String)> {
    let src = src.split("#[cfg(test)]\nmod tests").next().unwrap_or(src);
    let mut fns: Vec<(String, String)> = vec![];
    for line in src.lines() {
        let code = line.trim();
        if code.starts_with("//") {
            continue;
        }
        let declared = code
            .split_once("fn ")
            .filter(|(before, _)| before.split_whitespace().all(is_fn_qualifier))
            .and_then(|(_, rest)| rest.split_once('('))
            .map(|(name, _)| name.split('<').next().unwrap_or(name).to_string());
        if let Some(name) = declared {
            fns.push((name, String::new()));
        }
        if let Some((_, body)) = fns.last_mut() {
            body.extend(code.chars().filter(|c| !c.is_whitespace()));
        }
    }
    fns
}

/// A word that may come before `fn` in a declaration.
fn is_fn_qualifier(word: &str) -> bool {
    word == "async" || word == "const" || word.starts_with("pub")
}

/// Does this (whitespace-free) code lock the node connection? The window's node connection is
/// always reached as `db` (`state.db`, `self.db`, or a `db` bound from them); its other mutexes
/// (`session`, `shown`, `chart`, `funnel`) never are.
fn locks_the_node(code: &str) -> bool {
    code.contains("db.lock()")
        || code.contains("db.lock_if_open()")
        || code.match_indices(".db.as_ref()").any(|(at, _)| {
            let statement = &code[at..];
            let end = statement.find([';', '{']).unwrap_or(statement.len());
            statement[..end].contains(".lock()")
        })
}

#[test]
fn every_node_lock_explains_a_cut_off() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![];
    sources(&root.join("src"), &mut files);
    sources(&root.join("../cairn-gui-live/src"), &mut files);

    let mut locking = 0;
    let mut forgot = vec![];
    for (path, src) in &files {
        for (name, code) in functions(src) {
            if !locks_the_node(&code) || EXEMPT.contains(&name.as_str()) {
                continue;
            }
            locking += 1;
            if !EXPLAINS.iter().any(|e| code.contains(e)) {
                forgot.push(format!("{path}: fn {name}"));
            }
        }
    }
    // A floor, so a reader that stopped finding locks (a renamed binding, a new file layout)
    // fails here instead of passing with nothing checked.
    assert!(locking >= 20, "only {locking} node locks found");
    assert!(
        forgot.is_empty(),
        "these lock the node connection but never explain a cut-off (#893): pass the error \
         through `db.explain(e)`, or add the function to EXEMPT with why:\n{}",
        forgot.join("\n")
    );
}

/// The reader itself: it finds a lock in each shape the code uses, and only those.
#[test]
fn the_lock_reader_finds_each_shape() {
    let src = "pub async fn a(state: &S) {\n    let db = db.lock().await?;\n}\n\
               async fn b() {\n    state\n        .db\n        .as_ref()\n        .ok_or(\"x\")?\n        .lock()\n        .await?;\n}\n\
               pub(crate) fn c() {\n    match state.db.as_ref() {\n        None => state.chart.lock(),\n    }\n}\n";
    let found: Vec<(String, bool)> = functions(src)
        .into_iter()
        .map(|(name, code)| (name, locks_the_node(&code)))
        .collect();
    assert_eq!(
        found,
        vec![
            ("a".to_string(), true),
            ("b".to_string(), true),
            ("c".to_string(), false)
        ]
    );
}
