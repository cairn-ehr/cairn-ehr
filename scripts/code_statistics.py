#!/usr/bin/env python3
"""Walk the repository and report per-language line statistics.

For each source file we classify every line as one of: code, comment,
doc-comment (e.g. Rust `///`/`//!`, Python docstrings), or blank. Output
sections:

  1. Per-language summary table (files / code / comments / docs / blank / total).
  2. Overall totals.
  3. The longest files in the tree, so refactoring candidates are easy to spot.
  4. Documentation length per file (doc-comment / docstring lines, plus prose
     files like Markdown), sorted most-documented first.

"Code" excludes blank lines, comments, and inline docs. The scan skips build
output and, by relative path, `.claude/worktrees` (sibling checkouts) and
`docs/superpowers` (vendored content).

Standard-library only. Run from anywhere:

  ./scripts/code_statistics.py                 # scan repo root (default)
  ./scripts/code_statistics.py --root .        # explicit root
  ./scripts/code_statistics.py --top 30        # show 30 longest files
  ./scripts/code_statistics.py --min-lines 200 # only list files >= 200 lines
  ./scripts/code_statistics.py --json          # machine-readable output
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable


# ---------------------------------------------------------------------------
# Language definitions
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class LangDef:
    name: str
    extensions: tuple[str, ...] = ()
    filenames: tuple[str, ...] = ()           # exact basenames (e.g. "Makefile")
    line_comment: tuple[str, ...] = ()        # e.g. ("//",)
    doc_line_comment: tuple[str, ...] = ()    # e.g. ("///", "//!")
    block_comment: tuple[tuple[str, str], ...] = ()        # [(open, close), ...]
    doc_block_comment: tuple[tuple[str, str], ...] = ()    # e.g. [("/**", "*/")]
    # Python-style triple-quoted strings count as docs when they stand alone:
    triple_string: bool = False
    # Treat every non-blank line as documentation (Markdown, plain text):
    all_docs: bool = False


LANGS: tuple[LangDef, ...] = (
    LangDef(
        name="Rust",
        extensions=(".rs",),
        line_comment=("//",),
        doc_line_comment=("///", "//!"),
        block_comment=(("/*", "*/"),),
        doc_block_comment=(("/**", "*/"), ("/*!", "*/")),
    ),
    LangDef(
        name="Python",
        extensions=(".py", ".pyi"),
        line_comment=("#",),
        triple_string=True,
    ),
    LangDef(
        name="Shell",
        extensions=(".sh", ".bash", ".zsh"),
        filenames=(".bashrc", ".zshrc"),
        line_comment=("#",),
    ),
    LangDef(
        name="TOML",
        extensions=(".toml",),
        line_comment=("#",),
    ),
    LangDef(
        name="YAML",
        extensions=(".yaml", ".yml"),
        line_comment=("#",),
    ),
    LangDef(
        name="JSON",
        extensions=(".json",),
        # JSON has no comments; everything non-blank is "code".
    ),
    LangDef(
        name="SQL",
        extensions=(".sql",),
        line_comment=("--",),
        block_comment=(("/*", "*/"),),
    ),
    LangDef(
        name="JavaScript",
        extensions=(".js", ".mjs", ".cjs"),
        line_comment=("//",),
        block_comment=(("/*", "*/"),),
        doc_block_comment=(("/**", "*/"),),
    ),
    LangDef(
        name="TypeScript",
        extensions=(".ts", ".tsx"),
        line_comment=("//",),
        block_comment=(("/*", "*/"),),
        doc_block_comment=(("/**", "*/"),),
    ),
    LangDef(
        name="HTML",
        extensions=(".html", ".htm"),
        block_comment=(("<!--", "-->"),),
    ),
    LangDef(
        name="CSS",
        extensions=(".css",),
        block_comment=(("/*", "*/"),),
    ),
    LangDef(
        name="C/C++",
        extensions=(".c", ".h", ".cc", ".cpp", ".hpp", ".cxx"),
        line_comment=("//",),
        block_comment=(("/*", "*/"),),
        doc_block_comment=(("/**", "*/"),),
    ),
    LangDef(
        name="Go",
        extensions=(".go",),
        line_comment=("//",),
        block_comment=(("/*", "*/"),),
    ),
    LangDef(
        name="Makefile",
        extensions=(".mk",),
        filenames=("Makefile", "makefile", "GNUmakefile"),
        line_comment=("#",),
    ),
    LangDef(
        name="Dockerfile",
        filenames=("Dockerfile", "Containerfile"),
        line_comment=("#",),
    ),
    LangDef(
        name="Markdown",
        extensions=(".md", ".markdown"),
        all_docs=True,
    ),
    LangDef(
        name="reStructuredText",
        extensions=(".rst",),
        all_docs=True,
    ),
    LangDef(
        name="Text",
        extensions=(".txt",),
        all_docs=True,
    ),
)


def _build_lookup() -> tuple[dict[str, LangDef], dict[str, LangDef]]:
    by_ext: dict[str, LangDef] = {}
    by_name: dict[str, LangDef] = {}
    for lang in LANGS:
        for ext in lang.extensions:
            by_ext[ext.lower()] = lang
        for name in lang.filenames:
            by_name[name] = lang
    return by_ext, by_name


LANG_BY_EXT, LANG_BY_FILENAME = _build_lookup()


# ---------------------------------------------------------------------------
# Exclusions
# ---------------------------------------------------------------------------

# Directory names that never contain hand-written source we care about.
DEFAULT_SKIP_DIRS = frozenset({
    ".git",
    ".hg",
    ".svn",
    "target",            # Rust build output
    "build",
    "dist",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".idea",
    ".vscode",
    ".cargo",
    ".gradle",
    ".next",
    ".turbo",
})

# Specific files to skip even if extension matches (generated / vendored).
DEFAULT_SKIP_FILES = frozenset({
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
})

# Directories excluded by their path *relative to the scan root* (POSIX form),
# not just by basename. Use this for "docs/superpowers" so we don't
# accidentally skip an unrelated directory that merely shares a basename.
#
#   - ".claude/worktrees": sibling checkouts under this repo — analysing them
#     would double-count the whole tree. (Also pruned implicitly by the
#     dot-directory filter, but we name it here so it's excluded even with
#     --include-hidden.)
#   - "docs/superpowers": vendored skill content, not our code or docs.
DEFAULT_SKIP_REL_DIRS = frozenset({
    ".claude/worktrees",
    "docs/superpowers",
})


def detect_language(path: Path) -> LangDef | None:
    name = path.name
    if name in LANG_BY_FILENAME:
        return LANG_BY_FILENAME[name]
    ext = path.suffix.lower()
    return LANG_BY_EXT.get(ext)


# ---------------------------------------------------------------------------
# Per-file analysis
# ---------------------------------------------------------------------------

@dataclass
class FileStats:
    path: Path
    language: str
    total: int = 0
    code: int = 0
    comment: int = 0
    doc: int = 0
    blank: int = 0


@dataclass
class LangStats:
    name: str
    files: int = 0
    total: int = 0
    code: int = 0
    comment: int = 0
    doc: int = 0
    blank: int = 0

    def add(self, fs: FileStats) -> None:
        self.files += 1
        self.total += fs.total
        self.code += fs.code
        self.comment += fs.comment
        self.doc += fs.doc
        self.blank += fs.blank


# A line's classification — at most one bucket per line. Precedence:
#   blank > doc > comment > code.
# This means a `///` line is counted as doc even if it has code-looking tokens;
# a line with code AND a trailing `// comment` is counted as code (we don't
# split lines across categories — keeping it simple keeps the totals honest).

def _starts_with_any(stripped: str, markers: Iterable[str]) -> str | None:
    for m in markers:
        if stripped.startswith(m):
            return m
    return None


_TRIPLE_RE = re.compile(r'("""|\'\'\')')


def analyze_lines(lines: list[str], lang: LangDef) -> tuple[int, int, int, int]:
    """Return (code, comment, doc, blank)."""
    code = comment = doc = blank = 0

    in_block = False              # inside non-doc block comment
    in_doc_block = False          # inside doc block comment
    block_close: str | None = None
    triple_delim: str | None = None  # inside Python triple-quoted string

    for raw in lines:
        line = raw.rstrip("\n").rstrip("\r")
        stripped = line.strip()

        # --- Already inside a block comment / triple string ---------------
        if in_block or in_doc_block:
            assert block_close is not None
            if not stripped:
                blank += 1
            elif in_doc_block:
                doc += 1
            else:
                comment += 1
            if block_close in line:
                in_block = False
                in_doc_block = False
                block_close = None
            continue

        if triple_delim is not None:
            # Inside a Python triple-quoted string — count as doc.
            if not stripped:
                blank += 1
            else:
                doc += 1
            if triple_delim in line:
                triple_delim = None
            continue

        # --- Blank ---------------------------------------------------------
        if not stripped:
            blank += 1
            continue

        if lang.all_docs:
            doc += 1
            continue

        # --- Doc line comments (Rust ///, //!) ----------------------------
        if _starts_with_any(stripped, lang.doc_line_comment):
            doc += 1
            continue

        # --- Doc block comments (open on this line) -----------------------
        opened_doc_block = False
        for o, c in lang.doc_block_comment:
            if stripped.startswith(o):
                doc += 1
                if c in stripped[len(o):]:
                    # Opens and closes on the same line.
                    pass
                else:
                    in_doc_block = True
                    block_close = c
                opened_doc_block = True
                break
        if opened_doc_block:
            continue

        # --- Plain line comments ------------------------------------------
        if _starts_with_any(stripped, lang.line_comment):
            comment += 1
            continue

        # --- Plain block comments (open on this line) --------------------
        opened_block = False
        for o, c in lang.block_comment:
            if stripped.startswith(o):
                comment += 1
                if c in stripped[len(o):]:
                    pass
                else:
                    in_block = True
                    block_close = c
                opened_block = True
                break
        if opened_block:
            continue

        # --- Python triple-quoted string starting on this line ------------
        if lang.triple_string:
            m = _TRIPLE_RE.search(stripped)
            if m and stripped.startswith(m.group(0)):
                # Line begins with """ or ''' — treat as docstring.
                delim = m.group(0)
                rest = stripped[len(delim):]
                if delim in rest:
                    # Single-line docstring like: """foo"""
                    doc += 1
                else:
                    doc += 1
                    triple_delim = delim
                continue

        # --- Otherwise it's code ------------------------------------------
        code += 1

    return code, comment, doc, blank


def analyze_file(path: Path) -> FileStats | None:
    lang = detect_language(path)
    if lang is None:
        return None
    try:
        with path.open("r", encoding="utf-8", errors="strict") as fh:
            lines = fh.readlines()
    except (UnicodeDecodeError, OSError):
        return None  # binary or unreadable — skip silently

    code, comment, doc, blank = analyze_lines(lines, lang)
    total = code + comment + doc + blank
    return FileStats(
        path=path,
        language=lang.name,
        total=total,
        code=code,
        comment=comment,
        doc=doc,
        blank=blank,
    )


# ---------------------------------------------------------------------------
# Walking
# ---------------------------------------------------------------------------

def _rel_posix(root: Path, dirpath: str, name: str) -> str:
    """Path of `dirpath/name` relative to `root`, in POSIX form."""
    return (Path(dirpath) / name).relative_to(root).as_posix()


def iter_source_files(
    root: Path,
    skip_dirs: frozenset[str],
    skip_files: frozenset[str],
    skip_rel_dirs: frozenset[str] = DEFAULT_SKIP_REL_DIRS,
) -> Iterable[Path]:
    for dirpath, dirnames, filenames in os.walk(root):
        # Prune in-place so os.walk doesn't recurse into skipped dirs. A dir is
        # skipped if its basename is in skip_dirs, it is hidden, or its path
        # relative to the root matches an entry in skip_rel_dirs.
        dirnames[:] = [
            d for d in dirnames
            if d not in skip_dirs
            and not d.startswith(".")
            and _rel_posix(root, dirpath, d) not in skip_rel_dirs
        ]
        for fname in filenames:
            if fname in skip_files:
                continue
            if fname.startswith("."):
                continue
            yield Path(dirpath) / fname


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------

def print_table(headers: list[str], rows: list[list[str]]) -> None:
    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))

    def fmt(row: list[str]) -> str:
        parts = []
        for i, cell in enumerate(row):
            if i == 0:
                parts.append(cell.ljust(widths[i]))
            else:
                parts.append(cell.rjust(widths[i]))
        return "  ".join(parts)

    sep = "  ".join("-" * w for w in widths)
    print(fmt(headers))
    print(sep)
    for row in rows:
        print(fmt(row))


def report_text(
    files: list[FileStats],
    lang_totals: dict[str, LangStats],
    *,
    root: Path,
    top: int,
    min_lines: int,
) -> None:
    print(f"Code statistics for {root}")
    print()

    # --- Per-language table ---------------------------------------------
    lang_rows: list[list[str]] = []
    for lang in sorted(lang_totals.values(), key=lambda ls: ls.code, reverse=True):
        lang_rows.append([
            lang.name,
            f"{lang.files:,}",
            f"{lang.code:,}",
            f"{lang.comment:,}",
            f"{lang.doc:,}",
            f"{lang.blank:,}",
            f"{lang.total:,}",
        ])
    print_table(
        ["Language", "Files", "Code", "Comments", "Docs", "Blank", "Total"],
        lang_rows,
    )
    print()

    # --- Overall totals --------------------------------------------------
    totals = LangStats(name="TOTAL")
    for ls in lang_totals.values():
        totals.files += ls.files
        totals.code += ls.code
        totals.comment += ls.comment
        totals.doc += ls.doc
        totals.blank += ls.blank
        totals.total += ls.total

    print("Overall:")
    print(f"  Files    : {totals.files:,}")
    print(f"  Code     : {totals.code:,}")
    print(f"  Comments : {totals.comment:,}")
    print(f"  Docs     : {totals.doc:,}")
    print(f"  Blank    : {totals.blank:,}")
    print(f"  Total    : {totals.total:,}")
    if totals.code:
        ratio = (totals.comment + totals.doc) / totals.code
        print(f"  (comments + docs) / code = {ratio:.2f}")
    print()

    # --- Longest files (refactoring candidates) -------------------------
    longest = sorted(files, key=lambda f: f.total, reverse=True)
    longest = [f for f in longest if f.total >= min_lines]
    if top > 0:
        longest = longest[:top]

    if longest:
        header = f"Longest files (top {len(longest)}"
        if min_lines > 0:
            header += f", >= {min_lines} lines"
        header += "):"
        print(header)
        rows = [
            [
                str(fs.path.relative_to(root)) if fs.path.is_absolute() else str(fs.path),
                fs.language,
                f"{fs.total:,}",
                f"{fs.code:,}",
                f"{fs.comment + fs.doc:,}",
                f"{fs.blank:,}",
            ]
            for fs in longest
        ]
        print_table(
            ["File", "Language", "Total", "Code", "Cmt+Doc", "Blank"],
            rows,
        )
    else:
        print("No files matched the longest-files filter.")
    print()

    # --- Documentation length per file ----------------------------------
    # "Documentation" = doc-comment / docstring lines in source files, plus
    # every non-blank line of prose files (Markdown, rST, text). Sorted so the
    # most-documented files surface first.
    documented = sorted(
        (f for f in files if f.doc > 0),
        key=lambda f: f.doc,
        reverse=True,
    )
    if top > 0:
        documented = documented[:top]

    if documented:
        print(f"Documentation length per file (top {len(documented)} by doc lines):")
        rows = [
            [
                str(fs.path.relative_to(root)) if fs.path.is_absolute() else str(fs.path),
                fs.language,
                f"{fs.doc:,}",
                f"{fs.total:,}",
                f"{(fs.doc / fs.total * 100):.0f}%" if fs.total else "-",
            ]
            for fs in documented
        ]
        print_table(
            ["File", "Language", "Doc lines", "Total", "Doc %"],
            rows,
        )
    else:
        print("No documentation lines found.")


def report_json(
    files: list[FileStats],
    lang_totals: dict[str, LangStats],
    *,
    root: Path,
) -> None:
    payload = {
        "root": str(root),
        "languages": [
            {
                "name": ls.name,
                "files": ls.files,
                "code": ls.code,
                "comments": ls.comment,
                "docs": ls.doc,
                "blank": ls.blank,
                "total": ls.total,
            }
            for ls in sorted(lang_totals.values(), key=lambda x: x.code, reverse=True)
        ],
        "files": [
            {
                "path": str(fs.path.relative_to(root)) if fs.path.is_absolute() else str(fs.path),
                "language": fs.language,
                "total": fs.total,
                "code": fs.code,
                "comments": fs.comment,
                "docs": fs.doc,
                "blank": fs.blank,
            }
            for fs in sorted(files, key=lambda f: f.total, reverse=True)
        ],
    }
    json.dump(payload, sys.stdout, indent=2)
    sys.stdout.write("\n")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Per-language line statistics for the repository.",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="Repository root to scan (default: parent of this script's directory).",
    )
    parser.add_argument(
        "--top",
        type=int,
        default=20,
        help="Show this many longest files (0 = all). Default: 20.",
    )
    parser.add_argument(
        "--min-lines",
        type=int,
        default=0,
        help="Only list files with at least this many total lines in the 'longest files' table.",
    )
    parser.add_argument(
        "--skip-dir",
        action="append",
        default=[],
        help="Additional directory name to skip (may be repeated).",
    )
    parser.add_argument(
        "--include-hidden",
        action="store_true",
        help="Include files/directories whose name starts with a dot.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit JSON instead of the human-readable report.",
    )
    args = parser.parse_args(argv)

    if args.root is not None:
        root = args.root.resolve()
    else:
        # Default: parent of scripts/ directory == repo root.
        root = Path(__file__).resolve().parent.parent

    if not root.is_dir():
        print(f"error: {root} is not a directory", file=sys.stderr)
        return 2

    skip_dirs = DEFAULT_SKIP_DIRS | frozenset(args.skip_dir)
    if args.include_hidden:
        # Strip the dot-prefix filter inside iter_source_files by overriding
        # the function with a thin wrapper that keeps hidden entries.
        def walker():
            for dirpath, dirnames, filenames in os.walk(root):
                dirnames[:] = [
                    d for d in dirnames
                    if d not in skip_dirs
                    and _rel_posix(root, dirpath, d) not in DEFAULT_SKIP_REL_DIRS
                ]
                for fname in filenames:
                    if fname in DEFAULT_SKIP_FILES:
                        continue
                    yield Path(dirpath) / fname
        paths = walker()
    else:
        paths = iter_source_files(root, skip_dirs, DEFAULT_SKIP_FILES)

    files: list[FileStats] = []
    lang_totals: dict[str, LangStats] = {}
    for path in paths:
        fs = analyze_file(path)
        if fs is None:
            continue
        files.append(fs)
        lang_totals.setdefault(fs.language, LangStats(name=fs.language)).add(fs)

    if args.json:
        report_json(files, lang_totals, root=root)
    else:
        report_text(
            files,
            lang_totals,
            root=root,
            top=args.top,
            min_lines=args.min_lines,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
