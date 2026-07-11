-- db/030_john_doe_local_ordinal.sql
-- §5.4 node-local friendly John-Doe ordinal (a display aid, nothing more).
--
-- WHY: the callsign identity string (Unknown-<class>-<site>-<date>-<uuid-tail>) is
-- globally unique and partition-safe, but a UUID tail like "dead00ab" is not something a
-- clinician can say at the bedside. This VIEW derives a short, human-sayable
-- "this node's John Doe #N" handle from the immutable event_log. It NEVER touches the
-- callsign string, is never signed, never travels the wire, and is never an identity or a
-- merge key — so it cannot regress partition-safety.
--
-- HOW: row_number() PARTITIONs BY node_origin (the node that FIRST recorded the
-- registration), so each node numbers only the John Does it authored; a replicated foreign
-- registration lands in its own partition and never shifts this node's sequence. Ordering
-- within a partition is the collation-free (hlc_wall, hlc_counter, content_address) spine
-- (#115/#69): append-only log + monotonic single-node HLC means ranks never renumber, and
-- content_address (a BYTEA multihash, byte-ordered, identical on every node) breaks any
-- degenerate tie deterministically. All-time (no daily reset) — no timezone semantics to
-- get wrong.
--
-- WHAT IT SELECTS: exactly the callsign name authored by register_john_doe — a demographic
-- name assertion whose `use` facet is 'callsign' and whose provenance is the system
-- john-doe-registration marker. Never an ordinary name; never the pending marker (a
-- different event_type). event_log.body holds the event payload (see db/005 submit_event).
CREATE OR REPLACE VIEW john_doe_local_ordinal AS
SELECT patient_id,
       node_origin,
       body->>'value' AS callsign,
       row_number() OVER (PARTITION BY node_origin
                          ORDER BY hlc_wall, hlc_counter, content_address) AS ordinal
FROM event_log
WHERE event_type = 'demographic.field.asserted'
  AND body->>'field' = 'name'
  AND body->'facets'->>'use' = 'callsign'
  AND body->>'provenance' = 'system:john-doe-registration';
