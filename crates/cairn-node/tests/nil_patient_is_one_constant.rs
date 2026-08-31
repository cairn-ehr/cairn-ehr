//! The nil-patient UUID must have exactly ONE definition in the workspace.
//!
//! It moved to `cairn-event` (a wire constant belongs with the wire format) and
//! `cairn_node::identity` re-exports it. This guard exists because the failure mode of a
//! re-export quietly becoming a second literal is invisible: both spellings are the same
//! string today, so nothing breaks until someone edits one of them — and by then it is
//! inside signed bytes on two nodes that no longer agree.
//!
//! Pointer equality is the assertion, not string equality: two identical `&'static str`
//! literals may or may not be interned, but a genuine re-export is always the SAME
//! pointer. That is what distinguishes "re-exported" from "copied".

#[test]
fn node_identity_reexports_the_cairn_event_constant() {
    assert_eq!(
        cairn_node::identity::NIL_PATIENT,
        cairn_event::NIL_PATIENT,
        "the two paths must name the same value"
    );
    assert!(
        std::ptr::eq(
            cairn_node::identity::NIL_PATIENT.as_ptr(),
            cairn_event::NIL_PATIENT.as_ptr()
        ),
        "cairn_node::identity::NIL_PATIENT must RE-EXPORT cairn_event::NIL_PATIENT, \
         not re-declare an identical literal — a copy drifts silently"
    );
}
