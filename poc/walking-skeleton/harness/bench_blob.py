#!/usr/bin/env python3
"""Byte-tier throughput harness — Spike 0001 §8.2.

Drives the `cairn-sync` binary to validate the production byte tier: windowed +
resumable + multi-source swarm + per-slice BLAKE3 verification, without starving
clinical sync (the ADR-0013 availability floor). Stdlib only; `psql` is used for
setup (present on any node running PostgreSQL).

USE A RELEASE BINARY: cargo build --release.

WARNING: `selftest` DROPs and recreates the Cairn tables on the target DB(s).
Refuses to run without --force.

Checks:
  T1 windowed fetch    : a multi-MB blob fetches + verifies; report throughput + window
  T2 resume            : a fetch interrupted mid-transfer completes from persisted chunks
  T3 swarm             : chunks pulled from two honest sources still converge
  T4 lying peer        : a --corrupt source is rejected per-slice and healed by a good source
  T5 availability floor: clinical pull p95 unaffected during a concurrent windowed fetch
"""

import argparse
import json
import math
import os
import subprocess
import sys
import time
def p95(xs):
    if not xs:
        return 0.0
    s = sorted(xs)
    return s[min(len(s) - 1, int(round(0.95 * (len(s) - 1))))]


class Node:
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
             "drop table if exists event_log,hlc_state,sync_state,patient_chart,"
             "blob_store,blob_chunk cascade;"],
            capture_output=True, text=True,
        )

    def gen(self, key, patients=1, count=20, rate=0.0, background=False):
        return self._run("gen", "--node", self.name, "--key", key,
                         "--patients", str(patients), "--count", str(count),
                         "--rate", str(rate), background=background)

    def serve(self, corrupt=False):
        args = ["serve", "--listen", self.listen]
        if corrupt:
            args.append("--corrupt")
        return self._run(*args, background=True)

    def gen_blob(self, size_mb, media="application/dicom"):
        return self._json("gen-blob", "--size-mb", str(size_mb), "--media", media)

    def reference_blob(self, addr_hex, media, length):
        subprocess.run(
            ["psql", self.conn, "-qc",
             f"select blob_note_reference(decode('{addr_hex}','hex'),'{media}',{length});"],
            capture_output=True, text=True, check=True,
        )

    def blobd(self, peers, window=4, budget_ms=2, background=False):
        args = ["blobd", "--window", str(window), "--budget-ms", str(budget_ms), "--metrics"]
        for p in peers:
            args += ["--blob-peer", p]
        if background:
            return self._run(*args, background=True)
        return self._json(*args)

    def pull(self, peer_addr, peer_name):
        return self._json("pull", "--peer", peer_addr, "--peer-name", peer_name, "--metrics")

    def present(self, addr_hex):
        out = subprocess.run(
            ["psql", self.conn, "-tAc",
             f"select present, coalesce(octet_length(content),0) "
             f"from blob_store where blob_address=decode('{addr_hex}','hex');"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        if not out:
            return (False, 0)
        present, length = out.split("|")
        return (present == "t", int(length))

    def chunk_count(self, addr_hex):
        out = subprocess.run(
            ["psql", self.conn, "-tAc",
             f"select count(*) from blob_chunk where blob_address=decode('{addr_hex}','hex');"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        return int(out or 0)


def cmd_selftest(args):
    if not args.force:
        sys.exit("selftest is destructive (drops Cairn tables). Re-run with --force.")
    src = Node(args.bin, args.conn, "src", args.listen)
    src2 = Node(args.bin, args.conn_b, "src2", args.listen_b)
    dst = Node(args.bin, args.conn_c, "dst")
    for n in (src, src2, dst):
        n.reset()
        n.init()

    size_mb = args.size_mb
    nbytes = size_mb * 1024 * 1024
    media = "application/dicom"
    rows = []

    # Both honest sources hold the SAME blob (gen-blob is deterministic per-call only,
    # so generate on src and copy bytes to src2 via a file round-trip through put-blob).
    blob = src.gen_blob(size_mb, media)
    addr = blob["addr"]
    # Materialize identical bytes on src2 so it is a genuine second source: export
    # src's content as hex, write a file, put-blob it on src2 (same bytes -> same addr).
    tmp = f"/tmp/cairn_blob_{os.getpid()}.bin"
    keyfile = f"/tmp/cairn_floor_{os.getpid()}.key"
    hexout = subprocess.run(
        ["psql", src.conn, "-tAc",
         f"select encode(content,'hex') from blob_store where blob_address=decode('{addr}','hex')"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    with open(tmp, "wb") as f:
        f.write(bytes.fromhex(hexout))
    src2._run("put-blob", "--file", tmp, "--media", media)

    serves = [src.serve(), src2.serve()]
    src2_corrupt = None
    time.sleep(0.5)
    try:
        # T1 windowed fetch (single honest source).
        dst.reference_blob(addr, media, nbytes)
        t0 = time.time()
        m = dst.blobd([src.listen], window=args.window, budget_ms=args.budget_ms)
        # Loop passes until complete (a pass makes progress; resumable).
        passes = 1
        while not dst.present(addr)[0] and passes < 200:
            m = dst.blobd([src.listen], window=args.window, budget_ms=args.budget_ms)
            passes += 1
        elapsed = time.time() - t0
        present, length = dst.present(addr)
        mbps = (nbytes / (1 << 20)) / elapsed if elapsed > 0 else 0.0
        # Round-trip reduction metric (§4.6): compare old sequential stub vs windowed §8.2.
        OLD_CHUNK = 65536   # pre-§8.2 stub: one synchronous RTT per 64 KiB chunk
        SLICE = 262144      # MUST match SLICE_BYTES in cairn-sync (same constant as T2)
        seq_rtts = math.ceil(nbytes / OLD_CHUNK)
        n_slices = math.ceil(nbytes / SLICE)
        waves = math.ceil(n_slices / args.window)
        rows.append(("T1", "windowed fetch", present and length == nbytes,
                     f"{size_mb} MB in {elapsed:.1f}s ({mbps:.1f} MB/s), window {args.window}, {passes} pass(es)"
                     f"; ~{seq_rtts} seq RTTs (64KiB stub) -> ~{waves} windowed waves"
                     f" ({n_slices} slices/window {args.window})"))

        # T2 resume: INTERRUPT a fetch mid-transfer (a single blobd call drains the
        # whole queue, so resume only manifests on interruption), confirm a partial
        # set of chunks persisted, then resume to completion from those chunks. window=1
        # + 50ms budget makes the fetch take ~n_chunks*50ms so a 0.6s kill lands mid-way.
        SLICE = 262144  # MUST match SLICE_BYTES in cairn-sync
        t2_mb = max(size_mb, 8)
        t2_bytes = t2_mb * 1024 * 1024
        n_chunks = (t2_bytes + SLICE - 1) // SLICE
        t2_blob = src.gen_blob(t2_mb, media)
        t2_addr = t2_blob["addr"]
        # mirror the bytes onto src2 is unnecessary here (single source).
        dst.reset(); dst.init()
        dst.reference_blob(t2_addr, media, t2_bytes)
        bd = dst.blobd([src.listen], window=1, budget_ms=50, background=True)
        time.sleep(0.6)
        bd.terminate(); bd.wait()
        partial = dst.chunk_count(t2_addr)
        passes = 0
        while not dst.present(t2_addr)[0] and passes < 200:
            dst.blobd([src.listen], window=args.window, budget_ms=args.budget_ms)
            passes += 1
        resumed = dst.present(t2_addr)[0] and dst.present(t2_addr)[1] == t2_bytes
        rows.append(("T2", "resume across interrupt", resumed and 0 < partial < n_chunks,
                     f"{partial}/{n_chunks} chunks persisted at interrupt, then resumed to complete"))

        # T3 swarm: fresh dst, two honest sources.
        dst.reset(); dst.init()
        dst.reference_blob(addr, media, nbytes)
        passes = 0
        while not dst.present(addr)[0] and passes < 200:
            m = dst.blobd([src.listen, src2.listen], window=args.window, budget_ms=args.budget_ms)
            passes += 1
        rows.append(("T3", "swarm (2 sources)", dst.present(addr)[0] and dst.present(addr)[1] == nbytes,
                     f"converged from 2 sources in {passes} pass(es)"))

        # T4 lying peer: stop src2's honest serve, restart it as a CORRUPT source on the
        # same port; dst fetches from [liar, honest] so a rejected slice heals via src.
        serves[1].terminate(); serves[1].wait()
        time.sleep(0.3)
        src2_corrupt = src2.serve(corrupt=True)
        time.sleep(0.5)
        dst.reset(); dst.init()
        dst.reference_blob(addr, media, nbytes)
        rejected_total = 0
        passes = 0
        while not dst.present(addr)[0] and passes < 200:
            m = dst.blobd([src2.listen, src.listen], window=args.window, budget_ms=args.budget_ms)
            rejected_total += int(m["slices_rejected"])
            passes += 1
        healed = dst.present(addr)[0] and dst.present(addr)[1] == nbytes
        rows.append(("T4", "lying peer healed", healed and rejected_total > 0,
                     f"{rejected_total} slice(s) rejected by per-slice verify, then healed"))

        # T5 availability floor: clinical pull p95 unaffected during a windowed fetch.
        dst.reset(); dst.init()
        key = keyfile
        dst.reference_blob(addr, media, nbytes)

        def drain():
            for _ in range(200):
                if dst.pull(src.listen, src.name)["applied_new"] == 0:
                    return

        def sample():
            src.gen(key, patients=1, count=20)
            return dst.pull(src.listen, src.name)["elapsed_ms"]

        drain()
        base = [sample() for _ in range(args.rounds)]
        bd = dst.blobd([src.listen], window=args.window, budget_ms=args.budget_ms, background=True)
        during = []
        while bd.poll() is None and len(during) < args.rounds * 3:
            during.append(sample())
        bd.wait()
        base_p95, during_p95 = p95(base), p95(during)
        tol = args.tolerance
        floor_ok = during_p95 <= base_p95 * (1 + tol) + 5.0
        rows.append(("T5", "availability floor", floor_ok,
                     f"clinical pull p95 base {base_p95:.0f}ms -> during {during_p95:.0f}ms (tol {int(tol*100)}%)"))
    finally:
        for s in serves:
            s.terminate()
        if src2_corrupt:
            src2_corrupt.terminate()
        for f in (tmp, keyfile):
            try:
                os.remove(f)
            except OSError:
                pass

    # Render.
    w = max(len(r[1]) for r in rows)
    print(f"\n{'':4}{'check':<{w}}  result  detail")
    print("-" * (12 + w + 50))
    ok = True
    for code, name, passed, detail in rows:
        ok = ok and passed
        print(f"{code:<4}{name:<{w}}  {'PASS' if passed else 'FAIL':<6}  {detail}")
    print("-" * (12 + w + 50))
    print(f"\nByte tier: {'PASS' if ok else 'FAIL'}\n")
    sys.exit(0 if ok else 1)


def main():
    ap = argparse.ArgumentParser(description="Cairn byte-tier throughput harness (Spike 0001 §8.2)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    st = sub.add_parser("selftest", help="local multi-node validation (destructive)")
    st.add_argument("--bin", default="target/release/cairn-sync")
    st.add_argument("--conn", required=True, help="source node PG conn")
    st.add_argument("--conn-b", dest="conn_b", required=True, help="second source PG conn")
    st.add_argument("--conn-c", dest="conn_c", required=True, help="fetcher PG conn")
    st.add_argument("--listen", default="127.0.0.1:7790")
    st.add_argument("--listen-b", default="127.0.0.1:7791")
    st.add_argument("--size-mb", type=int, default=8)
    st.add_argument("--window", type=int, default=4)
    st.add_argument("--budget-ms", type=int, default=2)
    st.add_argument("--rounds", type=int, default=8)
    st.add_argument("--tolerance", type=float, default=0.30)
    st.add_argument("--force", action="store_true")
    st.set_defaults(func=cmd_selftest)
    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
