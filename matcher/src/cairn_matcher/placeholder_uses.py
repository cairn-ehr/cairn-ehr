"""§5.4 placeholder name-use registry — the single Python source of truth.

PURE: stdlib only, no psycopg, no I/O — so BOTH the psycopg-bound matching pipeline
(`pipeline/db.py`) and the pure synthetic-eval mirror (`eval/generator.py`) can import it
without either pulling the other's dependencies. This is the module the earlier "hoist
PLACEHOLDER_NAME_USES into a pure shared module" note (issue #124) called for: previously the
constant lived in `pipeline/db.py` (which imports psycopg at module top), so the pure eval
mirror could not reach it and re-declaring it there would have been a second hand-copy.

A "John Doe" chart carries a system-generated CALLSIGN as a real, displayed name; §5.4
requires the matcher EXCLUDE such placeholder names from its feature space (blocking AND
scoring) so two unidentified patients never match via their callsigns. The carrier is the
name's `use` facet (db/012 folds it to `use_key`); `callsign` is a system-set,
culture-neutral reserved token minted on the Rust side by `cairn-event::john_doe::CALLSIGN_USE`.

This set is the Python MIRROR of that Rust emitter, and the mirror is safety-relevant. Drift
is NOT recall-safe in the dangerous direction: a use the Rust side emits that is MISSING here
UNDER-excludes — those callsign names re-enter the feature space, and two same-site/same-day
John Does can then block+score+auto-band into a FALSE MERGE (§5.2's "false merge >> false
split"). So an addition on the Rust side MUST be mirrored here. `tests/test_placeholder_uses_sync.py`
reads the Rust constant and fails CI if it drifts out of this set — the mechanical guard that
replaces the old "documented on both sides, hope nobody forgets" coupling.
"""

# The one placeholder use minted today, kept as a named constant so the cross-language sync
# test can pin it to the Rust `CALLSIGN_USE` literal by name.
CALLSIGN_USE = "callsign"

# The reserved placeholder-use SET the matcher excludes. A SET (not a lone literal) so a
# future placeholder kind (e.g. a human-tagged `placeholder`) joins by adding one member —
# additive, never a rewrite. Every member MUST also be emitted/recognised on the Rust side.
PLACEHOLDER_NAME_USES = frozenset({CALLSIGN_USE})

# The parameter form psycopg binds to a Postgres `text[]` for `use_key <> ALL(%s)`. Sorted so
# the bound array is deterministic (readability / log stability); order is irrelevant to ALL.
PLACEHOLDER_USES_PARAM = sorted(PLACEHOLDER_NAME_USES)
