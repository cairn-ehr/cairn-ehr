"""Spike 0002 — the C1-C5 advisory-actor write-contract pass/fail table.

Self-contained selftest against ONE local database. Drives the agent stand-in
through submit_event and runs the five hostile-agent attacks; prints C1-C5 and
exits 0 iff all PASS. selftest DROPs+recreates the Cairn tables, so it requires
--force (guards a mistyped --conn), exactly like bet_a.py.
"""
import argparse
import hashlib
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
    # Relies on the caller's connection being autocommit=True (it is, in main): a
    # submit_event RAISE aborts the tx, so without autocommit the next statement
    # would fail with InFailedSqlTransaction rather than the needle we check for.
    try:
        db.execute(sql, params)
        return False, f"{label}: NO error raised (floor breached)"
    except psycopg.Error as e:
        msg = str(e)
        ok = needle.lower() in msg.lower()
        return ok, f"{label}: {'OK' if ok else 'WRONG ERROR'} — {msg.splitlines()[0]}"


def _content_address_hex(signed_hex):
    """The event's content address = 0x1220 (sha2-256 multihash prefix) + sha256 of
    the signed wire bytes — identical to event_address() in cairn-event and v_ca in
    db/005. Attestation tokens bind to THIS value.

    NOTE: this is the THIRD home of that encoding (Rust event_address, SQL v_ca, here).
    The three must move together — if the multihash prefix ever changes, update all
    three or this harness will silently mint wrong-address tokens. This copy exists
    only so the Python stand-in stays zero-dependency (no Rust call to derive the CA).
    """
    return "1220" + hashlib.sha256(bytes.fromhex(signed_hex)).hexdigest()


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
    rc, out, err = sh(args.bin, "init", "--conn", args.conn)
    if rc != 0:
        sys.exit(f"cairn-sync init failed: {err}")

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
            len(found) == 1 and str(found[0][0]) == str(eid)
            and n_after == n_before and aid_a != aid_b)

        # ---- C5: the hostile attacks all fail closed with legible reasons.
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
        # C5.6 impersonation: sign with the agent key but claim the human's
        # signer_key_id. signer_key_id is bound to the verifying key in-DB, so the
        # event fails the signature floor — an actor cannot author events attributed
        # to another (un-)enrolled actor. (Closes the Spike 0002 attribution-forgery gap.)
        impersonate = _agent_body("advisory.added", pid, {"x": 1},
                                  [{"alg":"blake3","digest_hex":blob_addr,"media_type":"m","descriptor":"d","byte_len":1}],
                                  human_key)  # claim the human's key id...
        signed = agent._sign(args.bin, "/tmp/agent.key", impersonate)  # ...sign with the agent key
        c5_checks.append(expect_raises(db, "SELECT submit_event(decode(%s,'hex'))", (signed,),
                                       "signature", "C5.6 impersonation (claimed signer_key_id)"))
        for ok, detail in c5_checks:
            print("   ", detail)
        # Committed-event set unchanged by the attacks (only the C1 advisory + patient exist).
        committed = db.execute("SELECT count(*) FROM event_log WHERE event_type='advisory.added'").fetchone()[0]
        results["C5 floor holds against hostile agent"] = all(ok for ok, _ in c5_checks) and committed == 1

        # ---- Attestation SUCCESS path (the C2 complement; closes the ADR-0030 gap).
        # A human attests the agent's suppressing event end-to-end -> accepted.
        supp2 = _agent_body("salience.downgrade", pid, {"target_event_id": str(eid)}, [], agent_key)
        supp2_signed = agent._sign(args.bin, "/tmp/agent.key", supp2)
        ca2 = _content_address_hex(supp2_signed)
        token2 = agent.attest(args.bin, "/tmp/human.key", ca2)
        row = db.execute(
            "SELECT submit_event(decode(%s,'hex'),decode(%s,'hex'),decode(%s,'hex'))",
            (supp2_signed, token2, human_key)).fetchone()
        accept_ok = row is not None and row[0] is not None
        print("   ", f"P  suppress + valid human token accepted: {accept_ok}")

        # A fresh suppress event with a WRONG-address token -> rejected.
        supp3 = _agent_body("salience.downgrade", pid, {"target_event_id": str(eid)}, [], agent_key)
        supp3_signed = agent._sign(args.bin, "/tmp/agent.key", supp3)
        wrong_token = agent.attest(args.bin, "/tmp/human.key", "1220" + "22" * 32)
        n1, d1 = expect_raises(
            db, "SELECT submit_event(decode(%s,'hex'),decode(%s,'hex'),decode(%s,'hex'))",
            (supp3_signed, wrong_token, human_key),
            "not bound to this event", "N1 wrong-address token rejected")
        print("   ", d1)

        # A correctly-bound token with one nibble flipped -> rejected.
        ca3 = _content_address_hex(supp3_signed)
        good3 = agent.attest(args.bin, "/tmp/human.key", ca3)
        flip = "0" if good3[len(good3) // 2] != "0" else "1"
        tampered = good3[:len(good3) // 2] + flip + good3[len(good3) // 2 + 1:]
        n2, d2 = expect_raises(
            db, "SELECT submit_event(decode(%s,'hex'),decode(%s,'hex'),decode(%s,'hex'))",
            (supp3_signed, tampered, human_key),
            "not bound to this event", "N2 tampered token rejected")
        print("   ", d2)

        results["Attestation success-path: accept + wrong-address/tamper rejected"] = (
            accept_ok and n1 and n2)

    print("\n  Spike 0002 — C1-C5")
    all_pass = True
    for k, v in results.items():
        print(f"  [{'PASS' if v else 'FAIL'}] {k}")
        all_pass = all_pass and v
    sys.exit(0 if all_pass else 1)


def _enroll(db, bin_path, kind, key_path, pinned):
    """Create the key (if absent), learn its real kid, enroll it, return the kid."""
    kid = agent.key_id(bin_path, key_path)
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
        except psycopg.Error as e:
            return False, f"C5.4 raw INSERT: WRONG error (not a privilege denial) — {str(e).splitlines()[0]}"
    finally:
        try:
            db.execute("RESET ROLE")
        except psycopg.Error:
            pass


if __name__ == "__main__":
    main()
