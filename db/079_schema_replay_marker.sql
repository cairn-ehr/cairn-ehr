-- db/079 -- a REPLAY MARKER, no objects (charting slice 1b, #1053).
--
-- db/075 changed IN PLACE (its authoring door now admits a `drugref-moiety` tolerated coding on a
-- de-label). A connect replays the schema only when SCHEMA_GENERATION moved (ADR-0080), and the
-- generation guard (crates/cairn-event/tests/schema_generation.rs) requires it to equal the newest
-- db/ prefix -- so an in-place edit needs a newest file. This is it; it changes nothing.
BEGIN;
SELECT 1;
COMMIT;
