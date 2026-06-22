"""Spike 0002 advisory-agent stand-in (fit-for-purpose Python, §9.1).

Mimics the kastellan integration *contract*: it loads its actor identity + skill
epoch, reads a provenance blob reference, computes a trivial urgency score, signs
the event with its own Ed25519 key (via `cairn-sync sign-stdin`, so Rust owns the
canonical COSE encoding), and authors the advisory ONLY through submit_event. It
never touches event_log directly.
"""
import json
import subprocess
import sys
import uuid

import psycopg


def _body(event_type, patient_id, schema, payload, attachments, kid):
    return {
        "event_id": str(uuid.uuid4()),  # UUIDv7 minted in Rust normally; v4 ok for the spike body
        "patient_id": patient_id,
        "event_type": event_type,
        "schema_version": schema,
        "hlc": {"wall": 1, "counter": 0, "node_origin": "agent"},
        "t_effective": None,
        "signer_key_id": kid,
        "contributors": [{"actor_id": "agent", "role": "triaged"}],
        "payload": payload,
        "attachments": attachments,
    }


def _sign(bin_path, key_path, body):
    p = subprocess.run([bin_path, "sign-stdin", "--key", key_path],
                       input=json.dumps(body).encode(), capture_output=True)
    if p.returncode != 0:
        raise RuntimeError(f"sign-stdin failed: {p.stderr.decode()}")
    return p.stdout.decode().strip()


def attest(bin_path, key_path, content_address_hex, role="attested"):
    """Mint a hex COSE_Sign1 attestation token bound to content_address_hex, signed
    by key_path's key, via `cairn-sync attest-stdin` (Rust owns the canonical encoding).

    Like _sign, this is a dumb signer: it attests whatever address it is handed, so a
    test can build a wrong-address token. The in-DB floor is what rejects a mis-binding.
    """
    # attest-stdin needs the attester's kid for the AttestationBody (unlike sign-stdin,
    # which derives the signer kid from the key itself).
    kid = key_id(bin_path, key_path)
    body = {"content_address_hex": content_address_hex, "attester_key_id": kid, "role": role}
    p = subprocess.run([bin_path, "attest-stdin", "--key", key_path],
                       input=json.dumps(body).encode(), capture_output=True)
    if p.returncode != 0:
        raise RuntimeError(f"attest-stdin failed: {p.stderr.decode()}")
    return p.stdout.decode().strip()


def key_id(bin_path, key_path):
    """Return the hex Ed25519 public key (kid) for key_path (creating it if absent).

    The body's signer_key_id must equal the signing key (the in-DB binding gate),
    so the agent learns its real kid up front rather than guessing.
    """
    p = subprocess.run([bin_path, "key-id", "--key", key_path], capture_output=True)
    if p.returncode != 0:
        raise RuntimeError(f"key-id failed: {p.stderr.decode()}")
    return p.stdout.decode().strip()


def author(conn_str, bin_path, key_path, blob_addr_hex, patient_id):
    """Author one advisory through submit_event. Returns the new event_id."""
    # Sign with — and declare — the kid this key actually owns (binding gate).
    kid = key_id(bin_path, key_path)
    with psycopg.connect(conn_str, autocommit=True) as db:
        # urgency score = a trivial deterministic function of the blob address.
        urgency = (int(blob_addr_hex[:2], 16) % 5) + 1
        body = _body(
            "advisory.added", patient_id, "advisory/1",
            {"urgency": urgency, "summary": "triage advisory (stand-in)"},
            [{"alg": "blake3", "digest_hex": blob_addr_hex,
              "media_type": "message/rfc822", "descriptor": "source mail", "byte_len": 1}],
            kid=kid,
        )
        signed_hex = _sign(bin_path, key_path, body)
        row = db.execute("SELECT submit_event(decode(%s,'hex'))", (signed_hex,)).fetchone()
        return row[0]


if __name__ == "__main__":
    USAGE = ("usage: agent_standin.py author --conn CONN --bin PATH "
             "--key PATH --blob-addr HEX --patient UUID")
    if len(sys.argv) < 2 or sys.argv[1] != "author":
        sys.exit(USAGE)
    args = dict(zip(sys.argv[2::2], sys.argv[3::2]))
    required = ["--conn", "--bin", "--key", "--blob-addr", "--patient"]
    missing = [k for k in required if k not in args]
    if missing:
        sys.exit(f"missing required args: {', '.join(missing)}\n{USAGE}")
    eid = author(args["--conn"], args["--bin"], args["--key"],
                 args["--blob-addr"], args["--patient"])
    print(eid)
