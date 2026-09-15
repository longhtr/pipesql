"""Stock builds and explicit artifacts shared by verification callers.

Standalone callers supply an owned build directory. Frozen gates supply one
identified stock build; callers verify it before linking or execution. Campaign cases,
oracles, environment changes, and result interpretation stay with their callers.
"""

from pathlib import Path
import argparse
import hashlib
import json
import runpy
import os
import sys

from check_process import run as run_process

ROOT = Path(__file__).resolve().parent.parent


def source_revision(root: Path) -> str | None:
    """Return optional Git context; frozen exports are identified by their manifest."""
    if not root.is_absolute():
        raise ValueError("source directory must be absolute")
    try:
        result = run_process(
            ["git", "rev-parse", "--verify", "HEAD"],
            cwd=root,
            timeout=30,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        return None
    if result.returncode < 0:
        result.check_returncode()
    return result.stdout.strip() if result.returncode == 0 else None


def require_artifact(path: Path) -> Path:
    """Reject missing, empty, or non-file inputs before linking or execution."""
    if not path.is_file() or path.stat().st_size == 0:
        raise ValueError(f"expected a nonempty artifact file: {path}")
    return path


def require_executable(path: Path) -> Path:
    """Validate a caller-supplied or freshly linked native executable."""
    require_artifact(path)
    if not os.access(path, os.X_OK):
        raise ValueError(f"artifact is not executable: {path}")
    return path


def _reserve_output(path: Path):
    """Claim a new output name, including against dangling symlinks."""
    if not path.is_absolute():
        raise ValueError("compiler output path must be absolute")
    with path.open("xb"):
        pass


def _build(work: Path, *, library_only: bool, timeout: float) -> Path:
    if not work.is_absolute():
        raise ValueError("build output directory must be absolute")
    target = work / "target"
    target.mkdir()  # A campaign cannot reuse another run's compilation artifacts.
    run_process(
        [
            "cargo",
            "build",
            "--release",
            "--offline",
            "--locked",
            *(["--lib"] if library_only else []),
            "--target-dir",
            str(target),
        ],
        cwd=ROOT,
        check=True,
        timeout=timeout,
    )
    release = target / "release"
    require_artifact(release / "libpipesql.rlib")
    if not library_only:
        require_executable(release / "pipesql")
    return release


def build_library(work: Path, *, timeout: float = 60) -> Path:
    """Use the gate's verified stock library, or build in a fresh target."""
    return shared_release() or _build(work, library_only=True, timeout=timeout)


def build_cli(work: Path, *, timeout: float = 90) -> Path:
    """Use the gate's verified stock CLI, or build in a fresh target."""
    return shared_release() or _build(work, library_only=False, timeout=timeout)


def dependency(release: Path, name: str) -> Path:
    """Require exactly one matching Rust dependency; never guess a revision."""
    matches = sorted((release / "deps").glob(f"lib{name}-*.rlib"))
    if len(matches) != 1:
        raise ValueError(
            f"expected one {name} rlib in {release / 'deps'}, found {len(matches)}"
        )
    return require_artifact(matches[0])


def native_library(work: Path, fixture: str, name: str) -> Path:
    """Build a native observer; case selection and fault state stay in its fixture."""
    source = require_artifact(ROOT / "tools/fixtures" / fixture)
    if sys.platform == "darwin":
        output = work / f"lib{name}.dylib"
        compiler = "clang"
        flags = ["-dynamiclib", f"-Wl,-install_name,{output}"]
    elif sys.platform == "linux":
        output = work / f"lib{name}.so"
        compiler = "cc"
        flags = [
            "-D_GNU_SOURCE", "-fPIC", "-shared",
            f"-Wl,-soname,lib{name}.so", "-ldl",
        ]
    else:
        raise ValueError("native observers require macOS or Linux")
    _reserve_output(output)
    run_process(
        [
            compiler,
            "-std=c11",
            "-O2",
            "-g",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wconversion",
            str(source),
            *flags,
            "-o",
            str(output),
        ],
        check=True,
        timeout=30,
        cwd=ROOT,
    )
    return require_artifact(output)


def observer_environment(library: Path) -> dict[str, str]:
    """Load this run's observer ahead of libc in the disposable stock caller."""
    if not library.is_absolute():
        raise ValueError("observer library path must be absolute")
    require_artifact(library)
    if ":" in str(library) or (
        sys.platform == "linux" and any(c.isspace() for c in str(library))
    ):
        raise ValueError("observer path contains a native loader list separator")
    if sys.platform == "darwin":
        variable = "DYLD_INSERT_LIBRARIES"
    elif sys.platform == "linux":
        variable = "LD_PRELOAD"
    else:
        raise ValueError("native observers require macOS or Linux")
    return {**os.environ, variable: str(library)}


def rust_driver(
    work: Path, release: Path, fixture: str, library: str, externs: tuple[str, ...] = ()
) -> Path:
    """Link a stock public-API caller and its explicitly named observer."""
    source = require_artifact(ROOT / "tools/fixtures" / fixture)
    require_artifact(release / "libpipesql.rlib")
    suffix = ".dylib" if sys.platform == "darwin" else ".so"
    require_artifact(work / f"lib{library}{suffix}")
    command = [
        "rustc",
        "--edition=2024",
        "-O",
        "-g",
        "-D",
        "warnings",
        str(source),
        "--extern",
        f"pipesql={release / 'libpipesql.rlib'}",
    ]
    for name in externs:
        command.extend(["--extern", f"{name}={dependency(release, name)}"])
    output = work / "driver"
    _reserve_output(output)
    command.extend(
        [
            "-L",
            f"dependency={release / 'deps'}",
            "-L",
            str(work),
            "-l",
            f"dylib={library}",
            "-o",
            str(output),
        ]
    )
    run_process(command, check=True, timeout=60, cwd=ROOT)
    return require_executable(output)


def artifact_manifest(release):
    """Identify all stock outputs, including dependencies used by rustc callers."""
    return {
        str(path.relative_to(release)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(release.rglob("*")) if path.is_file()
    }


def build_profile():
    source_manifest = runpy.run_path(str(ROOT / "tools/source-manifest.py"))["source_manifest"]
    compiler = run_process(
        ["rustc", "-vV"], cwd=ROOT, check=True, capture_output=True, text=True, timeout=30
    ).stdout
    return {
        "source_sha256": hashlib.sha256(source_manifest(ROOT).encode()).hexdigest(),
        "rustc": compiler,
        "profile": "release; default features; library and CLI",
        "environment": {
            name: value for name, value in sorted(os.environ.items())
            if (name.startswith("CARGO_") or name in (
                "RUSTFLAGS", "RUSTUP_TOOLCHAIN", "RUSTC", "RUSTC_WRAPPER",
                "RUSTC_WORKSPACE_WRAPPER",
            )) and name not in ("CARGO_TARGET_DIR", "CARGO_BUILD_JOBS")
        },
    }


def shared_release():
    """Fail closed on stale, incompatible or changed gate artifacts."""
    directory = os.environ.get("PIPESQL_STOCK_BUILD")
    if directory is None:
        return None
    work = Path(directory)
    if not work.is_absolute():
        raise ValueError("shared stock directory must be absolute")
    record = json.loads((work / "stock.json").read_text())
    release = work / "target/release"
    if record["profile"] != build_profile():
        raise ValueError("shared stock source or compiler profile differs")
    if record["artifacts"] != artifact_manifest(release):
        raise ValueError("shared stock artifacts changed")
    require_artifact(release / "libpipesql.rlib")
    require_executable(release / "pipesql")
    return release


def main(argv=None):
    parser = argparse.ArgumentParser(description="Build identified stock artifacts for a frozen gate.")
    parser.add_argument("output", type=Path)
    options = parser.parse_args(argv)
    if not options.output.is_absolute():
        parser.error("stock output must be absolute")
    options.output.mkdir()
    before = build_profile()
    release = _build(options.output, library_only=False, timeout=120)
    if before != build_profile():
        raise ValueError("stock source or compiler profile changed during build")
    record = {"profile": before, "artifacts": artifact_manifest(release)}
    for path in release.rglob("*"):
        if path.is_file():
            path.chmod(path.stat().st_mode & ~0o222)
    (options.output / "stock.json").write_text(json.dumps(record, indent=2) + "\n")


if __name__ == "__main__":
    main()
