-- Cairn — register the §5.4 identity-evidence event type (ADR-0042 photo evidence slice).
--
-- submit_event fails closed on an unregistered event_type (db/005), so a new type must add
-- its classification row before any node can author it. identity.evidence.asserted is
-- ADDITIVE (clinician-observed evidence never suppresses another author's content) and does
-- NOT target another author. No structural twin-floor branch is needed: it is non-demographic,
-- so db/015's cairn_event_twin carries its authored twin verbatim (ADR-0039).

BEGIN;

INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('identity.evidence.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

COMMIT;
