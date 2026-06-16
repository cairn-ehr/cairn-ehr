#!/usr/bin/env python3
"""Bet A measurement harness — Spike 0001 §5 (the WAN-sync bets).

Drives the `cairn-sync` binary to produce the §5 pass/fail table directly against
thresholds. Stdlib only (argparse/subprocess/json/statistics), so it runs on a
field MacBook or a Pi with no pip install.

Two ways to use it:

  * `selftest` — runs the whole §5 experiment on two local databases (two nodes on
    one machine). Self-contained; this is what CI / a smoke run exercises. Note A4
    is only *trivially* satisfiable single-box (no shared link to contend for) —
    its real test is the WAN run below; here it validates the mechanics.

  * the building blocks (`gen`, `converge`, `floor`, `fingerprint`, `report`) on a
    real node over WireGuard. The partition/latency injector is the link itself,
    optionally driven by `--partition-cmd` / `--heal-cmd` hooks (e.g. `wg-quick
    down wg0` / `up wg0`, or `tc qdisc` for added latency/loss).

§5 thresholds (overridable):
  A1 convergence : event_hash AND projection_hash identical across nodes
  A2 signatures  : zero verify-failures on apply
  A3 HLC/skew    : local clock merged past every applied event (skew reported, never auto-resolved)
  A4 floor       : clinical pull p95 during a concurrent blob fetch <= baseline p95 * tolerance
  A5 eager plane : bytes/event on the clinical plane <= budget
  A6 honest state: un-fetched blobs appear as referenced-but-not-present
"""

import argparse
import json
import os
import subprocess
import sys
import time
from statistics import median


def p95(xs):
    if not xs:
        return 0.0
    s = sorted(xs)
    return s[min(len(s) - 1, int(round(0.95 * (len(s) - 1))))]


class Node:
    """A handle to one cairn-sync node (a binary + a connection string)."""

    def __init__(self, bin_path, conn, name, listen=None):
        self.bin = bin_path
        self.conn = conn
        self.name = name
        self.listen = listen

    def _run(self, *args, background=False):
        cmd = [self.bin, args[0], "--conn", self.conn, *args[1:]]
        if background:
            return subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        out = subprocess.run(cmd, capture_output=True, text=True)
        if out.returncode != 0:
            raise RuntimeError(f"{' '.join(cmd)}\n{out.stderr.strip()}")
        return out.stdout

    def _json(self, *args):
        lines = [l for l in self._run(*args).splitlines() if l.strip().startswith("{")]
        return json.loads(lines[-1])

    def init(self):
        self._run("init")

    def reset(self):
        subprocess.run(
            ["psql", self.conn, "-qc",
             "drop table if exists event_log,hlc_state,sync_state,patient_chart,blob_store cascade;"],
            capture_output=True, text=True,
        )

    def gen(self, key, patients=10, count=100, rate=0.0, background=False):
        return self._run("gen", "--node", self.name, "--key", key,
                         "--patients", str(patients), "--count", str(count),
                         "--rate", str(rate), background=background)

    def serve(self):
        return self._run("serve", "--listen", self.listen, background=True)

    def pull(self, peer_addr, peer_name):
        return self._json("pull", "--peer", peer_addr, "--peer-name", peer_name, "--metrics")

    def blobd(self, peer_addr, budget_ms=5, background=False):
        return self._run("blobd", "--peer", peer_addr, "--budget-ms", str(budget_ms),
                        background=background)

    def put_blob(self, path, media):
        out = self._run("put-blob", "--file", path, "--media", media)
        # "stored blob <hex> (<len> bytes, ...)"
        addr = out.split()[2]
        return addr

    def reference_blob(self, addr_hex, media, length):
        subprocess.run(
            ["psql", self.conn, "-qc",
             f"select blob_note_reference(decode('{addr_hex}','hex'),'{media}',{length});"],
            capture_output=True, text=True, check=True,
        )

    def fingerprint(self):
        return self._json("fingerprint")


def converge(a: Node, b: Node, max_rounds=50):
    """Pull both directions until quiescent. Returns (verify_failures, bytes/event samples)."""
    vf, bpe = 0, []
    quiet = 0
    for _ in range(max_rounds):
        m1 = a.pull(b.listen, b.name)
        m2 = b.pull(a.listen, a.name)
        vf += m1["verify_failures"] + m2["verify_failures"]
        for m in (m1, m2):
            if m["shipped"]:
                bpe.append(m["bytes_per_event"])
        if m1["applied_new"] == 0 and m2["applied_new"] == 0:
            quiet += 1
            if quiet >= 2:
                break
        else:
            quiet = 0
    return vf, bpe


def floor_test(measurer: Node, source: Node, blob_mb, rounds, tolerance, budget_ms, batch=20):
    """A4: clinical pull p95 must not degrade while a big blob is fetched concurrently.

    Every sample is identical work — drain to caught-up, emit a *fixed* small batch
    on the source, time one pull — so baseline and during compare like for like (no
    free-running backlog feedback). The only difference between the two phases is
    the concurrent blob fetch.
    """
    blob = f"/tmp/cairn_floor_{os.getpid()}.bin"
    nbytes = blob_mb * 1024 * 1024
    with open(blob, "wb") as f:
        f.write(os.urandom(nbytes))
    addr = source.put_blob(blob, "application/dicom")
    measurer.reference_blob(addr, "application/dicom", nbytes)
    a6_referenced = measurer.fingerprint()["blobs_referenced_only"]  # A6 captured before fetch

    key = "/tmp/cairn_floor_src.key"

    def drain():
        for _ in range(200):
            if measurer.pull(source.listen, source.name)["applied_new"] == 0:
                return

    def sample():
        source.gen(key, patients=1, count=batch, rate=0.0)  # one fixed batch of clinical work
        return measurer.pull(source.listen, source.name)["elapsed_ms"]

    try:
        drain()
        base = [sample() for _ in range(rounds)]

        bd = measurer.blobd(source.listen, budget_ms=budget_ms, background=True)
        during = []
        while bd.poll() is None and len(during) < rounds * 3:
            during.append(sample())
        bd.wait()
    finally:
        try:
            os.remove(blob)
        except OSError:
            pass

    present = measurer.fingerprint()["blobs_present"]
    return {
        "p95_base_ms": round(p95(base), 1),
        "p95_during_ms": round(p95(during), 1),
        "median_base_ms": round(median(base), 1) if base else 0,
        "median_during_ms": round(median(during), 1) if during else 0,
        "tolerance": tolerance,
        "blob_fetched": present >= 1,
        "a6_referenced_only_before_fetch": a6_referenced,
    }


def render_table(rows):
    w = max(len(r[1]) for r in rows)
    print(f"\n{'':4}{'check':<{w}}  {'result':<6}  detail")
    print("-" * (10 + w + 40))
    ok = True
    for code, name, passed, detail in rows:
        ok = ok and passed
        print(f"{code:<4}{name:<{w}}  {'PASS' if passed else 'FAIL':<6}  {detail}")
    print("-" * (10 + w + 40))
    print(f"\nBet A: {'PASS — proceed to ratify the §4 primitives' if ok else 'FAIL — see failing rows'}\n")
    return ok


def cmd_selftest(args):
    a = Node(args.bin, args.conn_a, "node-a", args.listen_a)
    b = Node(args.bin, args.conn_b, "node-b", args.listen_b)

    for n in (a, b):
        n.reset()
        n.init()

    # Partition: each node writes independently (no link yet).
    a.gen("/tmp/cairn_a.key", patients=args.patients, count=args.notes)
    b.gen("/tmp/cairn_b.key", patients=args.patients, count=args.notes)

    serves = [a.serve(), b.serve()]
    time.sleep(1.0)
    try:
        # A1/A2/A5: reconnect and converge.
        verify_failures, bpe = converge(a, b)
        fa, fb = a.fingerprint(), b.fingerprint()

        # A4/A6: the availability-floor experiment (B fetches a blob from A).
        floor = floor_test(b, a, args.blob_mb, args.rounds, args.tolerance, args.budget_ms)
    finally:
        for s in serves:
            s.terminate()

    a1 = fa["event_hash"] == fb["event_hash"] and fa["projection_hash"] == fb["projection_hash"]
    a2 = verify_failures == 0
    a3 = fa["hlc_merged_past_max_event"] and fb["hlc_merged_past_max_event"]
    a4 = floor["blob_fetched"] and floor["p95_during_ms"] <= floor["p95_base_ms"] * args.tolerance
    avg_bpe = round(sum(bpe) / len(bpe)) if bpe else 0
    a5 = 0 < avg_bpe <= args.byte_budget
    a6 = floor["a6_referenced_only_before_fetch"] >= 1

    rows = [
        ("A1", "convergence (event + projection hash)", a1,
         f"events {fa['events']}={fb['events']}, hashes {'match' if a1 else 'DIFFER'}"),
        ("A2", "signatures survive the wire", a2, f"{verify_failures} verify-failures"),
        ("A3", "HLC merged / gap flagged", a3,
         f"max HLC-record gap A={fa['max_hlc_record_gap_ms']}ms B={fb['max_hlc_record_gap_ms']}ms (reported, not resolved)"),
        ("A4", "availability floor (blob vs clinical)", a4,
         f"p95 base {floor['p95_base_ms']}ms -> during {floor['p95_during_ms']}ms "
         f"(<= x{args.tolerance}); blob fetched={floor['blob_fetched']}"),
        ("A5", "eager plane slim (bytes/event)", a5,
         f"{avg_bpe} B/event (budget {args.byte_budget})"),
        ("A6", "honest assembly-state", a6,
         f"{floor['a6_referenced_only_before_fetch']} referenced-but-not-present before fetch"),
    ]
    print("\nNOTE: single-box selftest — A4 has no shared link to contend for, so it "
          "validates mechanics only.\n      The real A4 threshold is meaningful on the "
          "Cape York <-> Dorrigo WireGuard link.")
    ok = render_table(rows)
    sys.exit(0 if ok else 1)


def cmd_fingerprint(args):
    print(json.dumps(Node(args.bin, args.conn, args.name).fingerprint(), indent=2))


def cmd_report(args):
    """Compare two fingerprint JSON files captured on each node (two-machine A1/A3)."""
    fa = json.load(open(args.local))
    fb = json.load(open(args.peer))
    a1 = fa["event_hash"] == fb["event_hash"] and fa["projection_hash"] == fb["projection_hash"]
    a3 = fa["hlc_merged_past_max_event"] and fb["hlc_merged_past_max_event"]
    rows = [
        ("A1", "convergence (event + projection hash)", a1,
         f"local {fa['events']} ev / peer {fb['events']} ev"),
        ("A3", "HLC merged / gap flagged", a3,
         f"skew local={fa['max_hlc_record_gap_ms']}ms peer={fb['max_hlc_record_gap_ms']}ms"),
    ]
    sys.exit(0 if render_table(rows) else 1)


def cmd_analyze(args):
    """Summarise a `cairn-sync run` JSONL log into the §5 numbers for one node.

    A1 (convergence) needs both nodes — this prints the final fingerprint so you can
    save each node's and compare with `report`. A2/A4(latency)/A5/A6 + partition
    behaviour come from this single node's run.
    """
    rows = [json.loads(l) for l in open(args.log) if l.strip()]
    if not rows:
        print("empty log")
        sys.exit(1)

    cycles = len(rows)
    partitions = sum(1 for r in rows if r.get("partition"))
    dur_s = (rows[-1]["ts"] - rows[0]["ts"]) / 1000.0
    lat = [r["pull"]["elapsed_ms"] for r in rows if "pull" in r]
    vf = sum(r["pull"]["verify_failures"] for r in rows if "pull" in r)
    applied = sum(r["pull"]["applied_new"] for r in rows if "pull" in r)
    bpe = [r["pull"]["bytes_per_event"] for r in rows if r.get("pull", {}).get("shipped")]
    fps = [r["fingerprint"] for r in rows if "fingerprint" in r]
    final = fps[-1] if fps else {}

    print(f"\nrun log: {args.log}")
    print(f"  duration        {dur_s:.0f}s over {cycles} cycles")
    print(f"  partitions      {partitions} cycle(s) the peer was unreachable "
          f"({100*partitions/cycles:.0f}% of cycles)")
    print(f"  pull latency    p50 {median(lat):.0f}ms  p95 {p95(lat):.0f}ms  "
          f"max {max(lat):.0f}ms" if lat else "  pull latency    (no successful pulls)")
    print(f"  A2 verify-fails {vf}  ({'PASS' if vf == 0 else 'FAIL'})")
    print(f"  A5 bytes/event  {round(sum(bpe)/len(bpe)) if bpe else 0} "
          f"(over {len(bpe)} non-empty pulls)")
    print(f"  events applied  {applied}")
    if final:
        print(f"  A3 hlc merged   {final.get('hlc_merged_past_max_event')}  "
              f"(max HLC-record gap {final.get('max_hlc_record_gap_ms')}ms — flagged, not resolved)")
        print(f"  A6 blobs        {final.get('blobs_present')} present / "
              f"{final.get('blobs_referenced_only')} referenced-only")
        print(f"  final events    {final.get('events')}")
        # Save the final fingerprint for a two-node A1 comparison via `report`.
        out = args.log + ".fingerprint.json"
        json.dump(final, open(out, "w"))
        print(f"\n  -> final fingerprint written to {out}")
        print(f"     for A1, run: bet_a.py report --local {out} --peer <other-node>.fingerprint.json")
    print()


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    default_bin = os.path.join(here, "..", "target", "debug", "cairn-sync")

    ap = argparse.ArgumentParser(description="Bet A measurement harness (Spike 0001 §5)")
    ap.add_argument("--bin", default=default_bin, help="path to cairn-sync")
    sub = ap.add_subparsers(dest="cmd", required=True)

    st = sub.add_parser("selftest", help="run the whole §5 table on two local DBs")
    st.add_argument("--conn-a", required=True)
    st.add_argument("--conn-b", required=True)
    st.add_argument("--listen-a", default="127.0.0.1:7710")
    st.add_argument("--listen-b", default="127.0.0.1:7711")
    st.add_argument("--patients", type=int, default=20)
    st.add_argument("--notes", type=int, default=200)
    st.add_argument("--rounds", type=int, default=30)
    st.add_argument("--blob-mb", type=int, default=32)
    st.add_argument("--budget-ms", type=int, default=3)
    st.add_argument("--tolerance", type=float, default=1.5)
    st.add_argument("--byte-budget", type=int, default=4096)
    st.set_defaults(func=cmd_selftest)

    fp = sub.add_parser("fingerprint", help="print a node's convergence/honest-state JSON")
    fp.add_argument("--conn", required=True)
    fp.add_argument("--name", default="node")
    fp.set_defaults(func=cmd_fingerprint)

    rp = sub.add_parser("report", help="compare two fingerprint JSON files (two-machine A1/A3)")
    rp.add_argument("--local", required=True)
    rp.add_argument("--peer", required=True)
    rp.set_defaults(func=cmd_report)

    an = sub.add_parser("analyze", help="summarise a `cairn-sync run` JSONL log into §5 numbers")
    an.add_argument("--log", required=True)
    an.set_defaults(func=cmd_analyze)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
