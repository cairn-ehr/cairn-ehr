"""Cross-language drift guard for the §5.4 placeholder name-use set (issue #124).

The matcher excludes placeholder (callsign) names from its feature space using
`cairn_matcher.placeholder_uses.PLACEHOLDER_NAME_USES`. That set is the Python MIRROR of the
Rust emitter `cairn-event::john_doe::CALLSIGN_USE`, and the two are coupled only by hand.

The drift is SAFETY-RELEVANT and asymmetric:
  - a use the Rust side EMITS that is MISSING from the Python set UNDER-excludes — those
    callsign names re-enter the feature space and two same-site/same-day John Does can
    block+score+auto-band into a FALSE MERGE (§5.2's "false merge >> false split"); whereas
  - an EXTRA member on the Python side only over-excludes (reduced recall — safe).

So the load-bearing invariant is: *every placeholder use Rust emits must be in the Python
set.* This test reads the Rust constant from source and asserts exactly that, so a rename or
addition on the Rust side that is not mirrored here fails CI instead of silently shipping a
false-merge hazard. It is a pure test (reads a file, no DB, no psycopg); it self-skips only
when the Rust source is absent (a standalone/vendored matcher checkout).
"""

import re
from pathlib import Path

import pytest

from cairn_matcher.placeholder_uses import (
    CALLSIGN_USE,
    PLACEHOLDER_NAME_USES,
    PLACEHOLDER_USES_PARAM,
)

# tests/ -> matcher/ -> repo root; the Rust emitter lives in the cairn-event crate.
_RUST_JOHN_DOE = (
    Path(__file__).resolve().parents[2] / "crates" / "cairn-event" / "src" / "john_doe.rs"
)
_CALLSIGN_USE_RE = re.compile(r'pub const CALLSIGN_USE:\s*&str\s*=\s*"([^"]*)"')


def _rust_callsign_use() -> str:
    """Extract the string literal of Rust's `pub const CALLSIGN_USE` from source."""
    if not _RUST_JOHN_DOE.is_file():
        pytest.skip(f"Rust source absent ({_RUST_JOHN_DOE}) — standalone matcher checkout")
    m = _CALLSIGN_USE_RE.search(_RUST_JOHN_DOE.read_text(encoding="utf-8"))
    assert m is not None, (
        f"could not find `pub const CALLSIGN_USE: &str = \"...\"` in {_RUST_JOHN_DOE}; "
        f"the constant was renamed or reshaped — update this guard and PLACEHOLDER_NAME_USES."
    )
    return m.group(1)


def test_rust_callsign_use_is_mirrored_in_the_python_placeholder_set():
    rust_value = _rust_callsign_use()
    assert rust_value in PLACEHOLDER_NAME_USES, (
        f"Rust emits CALLSIGN_USE={rust_value!r} but it is NOT in the Python "
        f"PLACEHOLDER_NAME_USES={set(PLACEHOLDER_NAME_USES)!r}. This UNDER-excludes callsigns "
        f"and risks a FALSE MERGE of two John Does — add {rust_value!r} to "
        f"cairn_matcher.placeholder_uses.PLACEHOLDER_NAME_USES."
    )


def test_python_callsign_use_constant_matches_the_rust_literal():
    # Pin the exact literal on both sides so a rename on EITHER side trips CI.
    assert CALLSIGN_USE == _rust_callsign_use()


def test_placeholder_uses_param_is_the_sorted_set():
    # The bound-array form psycopg passes to `use_key <> ALL(%s)` must stay derived from the set.
    assert PLACEHOLDER_USES_PARAM == sorted(PLACEHOLDER_NAME_USES)
