#!/usr/bin/env python3
"""Hash build/gate inputs and reject uncovered Rust file inclusions.

Run from any directory. The output is a source manifest, not proof of a passing
build or a bit-reproducible binary. Parent revision and artifact hashes are still
required. Runtime benchmark inputs and observation drivers need separate records.
"""
import hashlib
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent
FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "README.md",
    "THIRD_PARTY.md",
)
TREES = (
    "src",
    "filesystem",
    "vendor",
    ".cargo",
    "tests",
    "tools",
    "docs",
    "notes",
    "examples",
)


def inputs(root):
    paths = {root / name for name in FILES}
    for name in TREES:
        paths.update(
            p
            for p in (root / name).rglob("*")
            if p.is_file() and "__pycache__" not in p.parts and p.name != ".DS_Store"
        )
    # File macros and explicit module paths must resolve to recorded inputs.
    # Fail rather than silently omitting compiled code or inventing a resolver
    # for generated/nonliteral paths; those need an explicit generator contract.
    for source in sorted(p for p in paths if p.suffix == ".rs"):
        text = source.read_text()
        calls = re.findall(r"\binclude(?:_(?:str|bytes))?\s*!\s*\(", text)
        literals = re.findall(
            r'\binclude(?:_(?:str|bytes))?\s*!\s*\(\s*"([^"\\]+)"\s*,?\s*\)', text
        )
        attributes = re.findall(r"#\s*\[\s*path\s*=", text)
        modules = re.findall(r'#\s*\[\s*path\s*=\s*"([^"\\]+)"\s*\]', text)
        if len(calls) != len(literals) or len(attributes) != len(modules):
            raise ValueError(
                f"nonliteral/unrecognized Rust inclusion in {source.relative_to(root)}"
            )
        for name in literals + modules:
            dependency = (source.parent / name).resolve()
            if dependency not in paths or not dependency.is_file():
                raise ValueError(
                    f"uncovered Rust input: {source.relative_to(root)} -> {name}"
                )
    return sorted(paths)


if __name__ == "__main__":
    for path in inputs(ROOT):
        print(
            f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(ROOT)}"
        )
