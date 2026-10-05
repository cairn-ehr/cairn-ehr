# matcher/src/cairn_matcher/cli.py
"""`cairn-matcher` — the advisory matcher's command line (repair path R4).

`cairn-matcher watch` runs the commit-time duplicate check: it drains db/056's notices and writes
match_proposal rows. It never links. Connect it as a role holding cairn_agent; with no --dsn the
standard libpq environment (PGHOST, PGPORT, PGUSER, PGDATABASE, …) is used.
"""

import argparse
import logging

from cairn_matcher.pipeline.watch import watch
from cairn_matcher.pipeline.worker import Settings


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="cairn-matcher")
    sub = p.add_subparsers(dest="cmd", required=True)
    w = sub.add_parser("watch", help="run the commit-time duplicate check")
    w.add_argument("--dsn", default="", help="libpq connection string (default: PG* env)")
    w.add_argument("--once", action="store_true", help="drain the queue once and exit")
    d = Settings()
    w.add_argument("--poll-seconds", type=float, default=d.poll_s)
    w.add_argument("--bulk-threshold", type=int, default=d.bulk_threshold)
    w.add_argument("--max-block-size", type=int, default=d.max_block_size)
    w.add_argument("--pace-ms", type=int, default=d.pace_ms)
    args = p.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(message)s")
    settings = Settings(poll_s=args.poll_seconds, bulk_threshold=args.bulk_threshold,
                        max_block_size=args.max_block_size, pace_ms=args.pace_ms)
    return watch(args.dsn, settings, once=args.once)


if __name__ == "__main__":
    raise SystemExit(main())
