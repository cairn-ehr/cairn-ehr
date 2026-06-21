"""Spike 0002 — the C1-C5 advisory-actor write-contract pass/fail table.

Self-contained selftest against ONE local database. Drives the agent stand-in
through submit_event and runs the five hostile-agent attacks; prints C1-C5 and
exits 0 iff all PASS. selftest DROPs+recreates the Cairn tables, so it requires
--force (guards a mistyped --conn), exactly like bet_a.py.
"""
import argparse
import json
import subprocess
import sys
import uuid

import psycopg
import agent_standin as agent

BIN_DEFAULT = "../target/debug/cairn-sync"


def sh(bin_path, *a, stdin=None):
    p = subprocess.run([bin_path, *a], input=stdin, capture_output=True)
    return p.returncode, p.stdout.decode(), p.stderr.decode()


def expect_raises(db, sql, params, needle, label):
    """Return True iff `sql` raises an error whose message contains `needle`."""
    try:
        db.execute(sql, params)
        return False, f"{label}: NO error raised (floor breached)"
    except psycopg.Error as e:
        msg = str(e)
        ok = needle.lower() in msg.lower()
        return ok, f"{label}: {'OK' if ok else 'WRONG ERROR'} — {msg.splitlines()[0]}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["selftest"])
    ap.add_argument("--conn", required=True)
    ap.add_argument("--bin", default=BIN_DEFAULT)
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()
    if not args.force:
        sys.exit("refusing to DROP/recreate without --force")

    # Fresh schema.
    with psycopg.connect(args.conn, autocommit=True) as db:
        for t in ["recall_overlay","event_type_class","blob_chunk","blob_store",
                  "patient_chart","actor_event","event_log","hlc_state","sync_state"]:
            db.execute(f"DROP TABLE IF EXISTS {t} CASCADE")
    sh(args.bin, "init", "--conn", args.conn)

    results = {}
    pid = str(uuid.uuid4())
    with psycopg.connect(args.conn, autocommit=True) as db:
        # Enroll a human attester and the agent (distinct keys).
        human_key = _enroll(db, args.bin, "human", "/tmp/human.key",
                            {"role": "clinician"})
        agent_key = _enroll(db, args.bin, "agent", "/tmp/agent.key",
                            {"model": "triage-stub", "version": "1", "skill_epoch": "epoch-a"})
        # A patient + a provenance blob the advisory can cite.
        db.execute("SELECT blob_note_reference(decode(%s,'hex'),%s,%s)",
                   ("1e20"+"11"*32, "message/rfc822", 1))
        blob_addr = "1e20" + "11"*32

        # ---- C1 + C3: the agent authors an additive, un-attested, provenance advisory.
        eid = agent.author(args.conn, args.bin, "/tmp/agent.key", blob_addr, pid)
        row = db.execute("SELECT contributors, attachments FROM event_log WHERE event_id=%s",
                         (eid,)).fetchone()
        contributors, attachments = row
        c1 = (any(c.get("role") == "triaged" and "responsibility" not in c for c in contributors)
              and not any("is_ai" in c for c in contributors))
        results["C1 additive, un-attested (no is_ai, no responsibility)"] = c1
        c3 = len(attachments) == 1 and attachments[0]["digest_hex"] == blob_addr
        results["C3 provenance-anchored"] = c3

        # ---- C2: an identical SUPPRESSING event authored un-attested is rejected.
        supp = _agent_body("salience.downgrade", pid, {"target_event_id": str(eid)}, [], agent_key)
        signed = agent._sign(args.bin, "/tmp/agent.key", supp)
        ok, detail = expect_raises(db, "SELECT submit_event(decode(%s,'hex'))", (signed,),
                                   "requires attestation", "C2 suppress-un-attested rejected")
        results["C2 additive accepted; suppressing-un-attested rejected"] = c1 and ok
        print("   ", detail)

        # ---- C4: recall query returns exactly this advisory; recall overlays, never erases.
        found = db.execute("SELECT event_id FROM events_by_actor_epoch(%s,%s)",
                           (agent_key, "epoch-a")).fetchall()
        n_before = db.execute("SELECT count(*) FROM event_log").fetchone()[0]
        db.execute("SELECT recall_event(%s,%s)", (str(eid), "epoch recall"))
        n_after = db.execute("SELECT count(*) FROM event_log").fetchone()[0]
        # Bumping skill_epoch mints a new actor_id (distinct from epoch-a's).
        aid_a = db.execute("SELECT cairn_actor_id(%s)",
                          (json.dumps({"model":"triage-stub","version":"1","skill_epoch":"epoch-a"}),)).fetchone()[0]
        aid_b = db.execute("SELECT cairn_actor_id(%s)",
                          (json.dumps({"model":"triage-stub","version":"1","skill_epoch":"epoch-b"}),)).fetchone()[0]
        results["C4 version-pinned + recallable (overlay, no erase)"] = (
            any(str(f[0]) == str(eid) for f in found) and n_after == n_before and aid_a != aid_b)

        # ---- C5: the five hostile attacks all fail closed with legible reasons.
        c5_checks = []
        # C5.1 unsigned/malformed
        c5_checks.append(expect_raises(db, "SELECT submit_event(%s)", (b"\xde\xad",),
                                       "signature", "C5.1 unsigned/malformed"))
        # C5.4 raw INSERT as the agent role
        c5_checks.append(_raw_insert_denied(db))
        # C5.2 forged human author (responsibility claimed, no token)
        forged = _agent_body("advisory.added", pid, {"x": 1},
                             [{"alg":"blake3","digest_hex":blob_addr,"media_type":"m","descriptor":"d","byte_len":1}],
                             agent_key, responsibility="attested")
        signed = agent._sign(args.bin, "/tmp/agent.key", forged)
        c5_checks.append(expect_raises(db, "SELECT submit_event(decode(%s,'hex'))", (signed,),
                                       "attestation", "C5.2 forged human author"))
        # C5.3 == C2 (suppress-un-attested) already covered; re-assert here for the table.
        c5_checks.append((results["C2 additive accepted; suppressing-un-attested rejected"],
                          "C5.3 suppressing-un-attested (see C2)"))
        # C5.5 salience downgrade of another author's event, un-attested
        downgrade = _agent_body("salience.downgrade", pid, {"target_event_id": str(eid)}, [], agent_key)
        signed = agent._sign(args.bin, "/tmp/agent.key", downgrade)
        c5_checks.append(expect_raises(db, "SELECT submit_event(decode(%s,'hex'))", (signed,),
                                       "attestation", "C5.5 cross-author salience downgrade"))
        for ok, detail in c5_checks:
            print("   ", detail)
        # Committed-event set unchanged by the attacks (only the C1 advisory + patient exist).
        committed = db.execute("SELECT count(*) FROM event_log WHERE event_type='advisory.added'").fetchone()[0]
        results["C5 floor holds against hostile agent"] = all(ok for ok, _ in c5_checks) and committed == 1

    print("\n  Spike 0002 — C1-C5")
    all_pass = True
    for k, v in results.items():
        print(f"  [{'PASS' if v else 'FAIL'}] {k}")
        all_pass = all_pass and v
    sys.exit(0 if all_pass else 1)


def _enroll(db, bin_path, kind, key_path, pinned):
    """Create the key (sign a throwaway body), learn its kid, enroll it, return the kid."""
    body = _agent_body("probe.added", str(uuid.uuid4()), {}, [], "")
    signed = agent._sign(bin_path, key_path, body)
    kid = db.execute("SELECT cairn_body(decode(%s,'hex')) ->> 'signer_key_id'", (signed,)).fetchone()[0]
    db.execute("SELECT enroll_actor(%s,%s,%s)", (kind, json.dumps(pinned), kid))
    return kid


def _agent_body(event_type, patient_id, payload, attachments, kid, responsibility=None):
    contrib = {"actor_id": "agent", "role": "triaged"}
    if responsibility:
        contrib = {"actor_id": "agent", "role": "attested", "responsibility": responsibility}
    return {
        "event_id": str(uuid.uuid4()), "patient_id": patient_id,
        "event_type": event_type, "schema_version": "advisory/1",
        "hlc": {"wall": 1, "counter": 0, "node_origin": "agent"},
        "t_effective": None, "signer_key_id": kid,
        "contributors": [contrib], "payload": payload, "attachments": attachments,
    }


def _raw_insert_denied(db):
    try:
        db.execute("SET ROLE cairn_agent")
        try:
            db.execute("""INSERT INTO event_log (event_id,patient_id,event_type,schema_version,
                hlc_wall,hlc_counter,node_origin,signed_bytes,content_address,body,contributors,
                signer_key_id,plaintext_twin) VALUES (gen_random_uuid(),gen_random_uuid(),'x','x',
                0,0,'n','\\x00','\\x1220'||digest('\\x00','sha256'),'{}','[]','k','t')""")
            db.execute("RESET ROLE")
            return False, "C5.4 raw INSERT: NOT denied (floor breached)"
        except psycopg.errors.InsufficientPrivilege as e:
            return True, f"C5.4 raw INSERT denied — {str(e).splitlines()[0]}"
    finally:
        try:
            db.execute("RESET ROLE")
        except psycopg.Error:
            pass


if __name__ == "__main__":
    main()
