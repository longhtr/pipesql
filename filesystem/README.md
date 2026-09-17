# Native filesystem boundary

This private crate supplies filesystem and mutex operations to the database through
safe Rust interfaces. It supports macOS and GNU/Linux. Start with
[src/lib.rs](src/lib.rs) for the operations and their ownership rules.

## Structure

| Area | Responsibility |
| --- | --- |
| [src/lib.rs](src/lib.rs) | File ownership, bounded directory traversal, path scratch storage and the crate's public interface. |
| [src/metadata.rs](src/metadata.rs) | Put native and Rust metadata into a common representation for identity checks. |
| [src/mutex.rs](src/mutex.rs) | Keep native mutex storage at a fixed address and expose guarded access to a Rust value. |
| [src/syscall.rs](src/syscall.rs) | Prepare native arguments, call the OS and translate results into owned handles or errors. |
| [src/syscall/path.rs](src/syscall/path.rs) | Resolve macOS paths with bounded work and storage. |
| [src/syscall/linux_path.rs](src/syscall/linux_path.rs) | Resolve Linux paths, using caller-accounted scratch storage when needed. |
| [src/tests.rs](src/tests.rs) | Check file, directory and path behavior across the safe interface. |

## Follow an operation

Opening a file transfers an owned `File` to the caller. The database then checks
its type and identity before trusting its contents. Path resolution and metadata
are observations; neither prevents another process from replacing a name later.

The database decides when publication requires synchronization and how to recover
from failure. This crate reports native errors without substituting weaker
synchronization. It cannot decide whether a transaction committed or which files
recovery may delete. See [architecture](../docs/architecture.md) for those owners and
[platform limitations](../docs/operations.md#choose-storage-that-meets-the-engines-assumptions) for storage qualification.

The `test-stack-observation` feature exposes native stack extents for tests; it is
not part of the ordinary library interface. Build and verification commands live
in [DEVELOPMENT.md](../DEVELOPMENT.md).
