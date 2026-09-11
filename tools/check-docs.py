#!/usr/bin/env python3
"""Check repository Markdown links and local heading anchors without dependencies."""

from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parent.parent


def prose(text):
    """Exclude fenced examples, whose apparent links are not document links."""
    result = []
    fence = None
    for line in text.splitlines():
        match = re.match(r"^\s{0,3}(`{3,}|~{3,})", line)
        if match:
            marker = match[1]
            if fence is None:
                fence = marker
            elif marker[0] == fence[0] and len(marker) >= len(fence):
                fence = None
            result.append("")
        else:
            result.append(line if fence is None else "")
    return "\n".join(result)


def anchors(text):
    text = prose(text)
    found = set(re.findall(r"\b(?:id|name)=[\"\x27]([^\"\x27]+)[\"\x27]", text))
    used = set()
    lines = text.splitlines()
    for index, line in enumerate(lines):
        heading = re.match(r"^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$", line)
        title = heading[1] if heading else None
        if title is None and index and re.fullmatch(r"\s{0,3}(?:=+|-+)\s*", line):
            title = lines[index - 1].strip()
        if not title:
            continue
        title = re.sub(r"<[^>]+>", "", title)
        title = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", title)
        slug = re.sub(r"[^\w\- ]", "", title.lower()).replace(" ", "-")
        candidate = slug
        suffix = 0
        while candidate in used:
            suffix += 1
            candidate = f"{slug}-{suffix}"
        used.add(candidate)
        found.add(candidate)
    return found


def links(text):
    text = prose(text)
    definitions = {}
    for match in re.finditer(r"^\s{0,3}\[([^\]]+)\]:\s*(<[^>]+>|\S+)", text, re.M):
        definitions[match[1].strip().casefold()] = match[2].strip("<>")
    pattern = (
        r"!?\[[^\]]*\]\(\s*(<[^>]+>|[^\s)]+)(?:\s+[\"\x27][^\n]*?[\"\x27])?\s*\)"
    )
    for match in re.finditer(pattern, text):
        yield text.count("\n", 0, match.start()) + 1, match[1].strip("<>")
    for match in re.finditer(r"!?\[([^\]\n]+)\]\[([^\]\n]*)\]", text):
        name = (match[2] or match[1]).strip().casefold()
        yield text.count("\n", 0, match.start()) + 1, definitions.get(
            name, f"missing-reference:{name}"
        )


def check(root, documents):
    root = root.resolve()
    failures = []
    count = 0
    heading_cache = {}
    for document in documents:
        document = document.resolve()
        for line, target in links(document.read_text()):
            location = f"{document.relative_to(root)}:{line}"
            if target.startswith("missing-reference:"):
                failures.append(f"{location}: undefined {target}")
                continue
            url = urlsplit(target)
            if url.scheme or url.netloc:
                continue
            count += 1
            path = (
                (document.parent / unquote(url.path)).resolve()
                if url.path
                else document.resolve()
            )
            if not path.is_relative_to(root):
                failures.append(f"{location}: link escapes repository: {target}")
            elif not path.exists():
                failures.append(f"{location}: missing target: {target}")
            elif url.fragment and path.suffix.lower() == ".md":
                if path not in heading_cache:
                    heading_cache[path] = anchors(path.read_text())
                if unquote(url.fragment) not in heading_cache[path]:
                    failures.append(f"{location}: missing anchor: {target}")
    return count, failures


def documents(root):
    paths = list(root.glob("*.md"))
    for directory in ("docs", "notes", "tests", "tools"):
        paths.extend((root / directory).rglob("*.md"))
    # README is the root entry point; maintained guides live in the trees above.
    tracked_root_docs = {"README.md", "THIRD_PARTY.md"}
    return sorted(
        path for path in paths if path.parent != root or path.name in tracked_root_docs
    )


def main():
    paths = documents(ROOT)
    count, failures = check(ROOT, paths)
    for failure in failures:
        print(failure, file=sys.stderr)
    print(f"documents={len(paths)} local_links={count} failures={len(failures)}")
    return bool(failures)


if __name__ == "__main__":
    sys.exit(main())
