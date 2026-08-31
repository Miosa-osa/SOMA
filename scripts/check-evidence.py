#!/usr/bin/env python3
"""Checks that the documentation corpus can be verified by someone who did not write it.

Four rules, each of which failed in the corpus at least once:

  links      every relative link under docs/ resolves to a file or directory that exists
  ledger     every live-proved claim-ledger row names a commit that is in history, and links
             at least one retained evidence artifact
  retention  every evidence document that quotes a time figure retains a sample: a link into
             docs/evidence/raw/, or a transcript fenced in the document itself
  orphans    every directory under docs/evidence/raw/ is cited by some document

What this deliberately does not check is written in docs/evidence/README.md.

Usage: check-evidence.py [docs-directory]
"""

import pathlib
import re
import subprocess
import sys

FENCE = re.compile(r"^\s*```")
LINK = re.compile(r"\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
# A time figure. Bare "s" is excluded: it collides with ordinary prose far too often to be a
# usable signal, and every document that quotes seconds also quotes milliseconds.
FIGURE = re.compile(r"\b\d+(?:\.\d+)?\s*(?:ms|us|µs|ns)\b")
LEDGER_ROW = re.compile(r"^\|")
LIVE_PROVED = re.compile(r"Live-proved(?:\s+at\s+`([0-9a-f]{7,40})`)?")
SKIP_SCHEMES = ("http://", "https://", "mailto:", "ftp://")


def strip_fences(text):
    """Returns (prose, had_fence). Links inside a transcript are examples, not references."""
    lines, inside, had_fence = [], False, False
    for line in text.splitlines():
        if FENCE.match(line):
            inside = not inside
            had_fence = True
            continue
        lines.append("" if inside else line)
    return "\n".join(lines), had_fence


def relative_links(prose):
    for target in LINK.findall(prose):
        if target.startswith(SKIP_SCHEMES) or target.startswith("#"):
            continue
        yield target


def check_links(docs, failures):
    for path in sorted(docs.rglob("*.md")):
        prose, _ = strip_fences(path.read_text(encoding="utf-8"))
        for target in relative_links(prose):
            resolved = (path.parent / target.split("#", 1)[0].rstrip("/")).resolve()
            if not resolved.exists():
                failures.append(f"{path}: links to {target}, which resolves to nothing")


def history_has(commit):
    try:
        subprocess.run(
            ["git", "cat-file", "-e", f"{commit}^{{commit}}"],
            check=True, capture_output=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return False
    return True


def repository_state(root):
    """Returns None when history can be trusted, or a message saying why it cannot."""
    try:
        inside = subprocess.run(
            ["git", "rev-parse", "--is-inside-work-tree"],
            cwd=root, check=True, capture_output=True, text=True,
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "not a git checkout, so no ledger commit can be resolved"
    if inside != "true":
        return "not a git work tree, so no ledger commit can be resolved"
    shallow = subprocess.run(
        ["git", "rev-parse", "--is-shallow-repository"],
        cwd=root, check=False, capture_output=True, text=True,
    ).stdout.strip()
    if shallow == "true":
        return ("the checkout is shallow, so a commit that exists would still look missing; "
                "check out with fetch-depth: 0")
    return None


def check_ledger(ledger, failures):
    if not ledger.exists():
        failures.append(f"{ledger}: missing, and every capability claim needs a ledger row")
        return
    state = repository_state(ledger.parent.parent)
    prose, _ = strip_fences(ledger.read_text(encoding="utf-8"))
    for number, line in enumerate(prose.splitlines(), start=1):
        if not LEDGER_ROW.match(line):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 3 or not cells[1].startswith("Live-proved"):
            continue
        where = f"{ledger}:{number}: the row for {cells[0]!r}"
        commits = LIVE_PROVED.findall(cells[1])
        named = [commit for commit in commits if commit]
        if not named:
            failures.append(f"{where} is live-proved and names no commit")
        elif state is not None:
            failures.append(f"{where} names {named[0]}, which cannot be checked: {state}")
        else:
            for commit in named:
                if not history_has(commit):
                    failures.append(f"{where} names commit {commit}, which is not in history")
        targets = list(relative_links(cells[2]))
        if not targets:
            failures.append(f"{where} is live-proved and links no retained evidence")


def check_retention(evidence, failures):
    raw = evidence / "raw"
    for path in sorted(evidence.glob("*.md")):
        if path.name == "README.md":
            continue
        text = path.read_text(encoding="utf-8")
        prose, had_fence = strip_fences(text)
        if not FIGURE.search(prose):
            continue
        retained = any(
            (path.parent / target.split("#", 1)[0].rstrip("/")).resolve() == raw
            or raw in (path.parent / target.split("#", 1)[0].rstrip("/")).resolve().parents
            for target in relative_links(prose)
        )
        if not retained and not had_fence:
            failures.append(
                f"{path}: quotes a time figure and retains no sample; it links nothing under "
                f"{raw} and holds no transcript"
            )


def check_orphans(docs, failures):
    raw = docs / "evidence" / "raw"
    if not raw.is_dir():
        return
    cited = set()
    for path in docs.rglob("*.md"):
        prose, _ = strip_fences(path.read_text(encoding="utf-8"))
        for target in relative_links(prose):
            resolved = (path.parent / target.split("#", 1)[0].rstrip("/")).resolve()
            for candidate in (resolved, *resolved.parents):
                if candidate.parent == raw:
                    cited.add(candidate.name)
    for record in sorted(raw.iterdir()):
        if record.is_dir() and record.name not in cited:
            failures.append(
                f"{record}: retained and cited by no document, so nothing can be checked "
                f"against it; link it from the record that used it or delete it"
            )


def main(argv):
    docs = pathlib.Path(argv[1] if len(argv) > 1 else "docs").resolve()
    if not docs.is_dir():
        print(f"check-evidence: {docs} is not a directory", file=sys.stderr)
        return 2
    failures = []
    check_links(docs, failures)
    check_ledger(docs / "claim-ledger.md", failures)
    check_retention(docs / "evidence", failures)
    check_orphans(docs, failures)
    if failures:
        print(f"check-evidence: {len(failures)} problems", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print("evidence checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
