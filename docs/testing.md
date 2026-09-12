# Build and test

This guide owns setup and commands. [Verification](verification.md) owns
coverage requirements and permitted claims; [the work plan](../notes/plan.md)
identifies the current work. [Retained evidence](../notes/evidence.md)
identifies completed runs and their exact inputs. This page does not claim a
passing run.

## Prerequisites

Use the pinned Rust 1.98.1 toolchain with Clippy and rustfmt. The repository
also requires Python 3.11 or newer with assertions enabled, `/usr/bin/time` for
ownership observations, and a native C compiler and SDK for ABI and independent
native checks. On macOS, install the command-line SDK tools before running those
checks. Python helpers use the standard library.

The toolchain must be provisioned before offline verification. Cargo
dependencies are locked and libc is vendored; do not replace them with network
resolution. The reviewed macOS path has the broadest runtime coverage. A native
Linux subset also passes; it does not qualify the missing integration and
durability checks. Other Unix targets are rejected. Read the platform exclusions
below before interpreting a successful test run.

Run commands below from the repository root unless a command says otherwise.

## Platform status

PipeSQL targets macOS, Linux, and Windows. This table separates available code
from checks actually performed; it is not a release-support promise.

| Boundary | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Rust library/CLI and test compilation | Native arm64 release builds and Clippy exercised. | x86_64 GNU all-target cross-check and arm64 GNU native release tests pass with warnings denied. | Blocked by Unix imports and the missing native implementation. |
| Filesystem effects | Native path, metadata, directory, locking, and synchronization implementations; scoped failure campaigns exercised. | Native filesystem tests exercise the implemented boundary. Synchronization, byte-I/O failure, and catalog interruption campaigns are implemented and exercised on GNU arm64. Path traversal has explicit resource bounds; durability qualification remains unfinished. | No implementation yet. |
| Declared-table lifecycle and queries | Public integration and failure tests exercised; full release qualification remains open. | Public catalog integration and bounded-thread scenarios run on Linux with explicit target ceilings; see below. | Cannot build until the native boundary is implemented. |
| Legacy lineitem loader | Available on the reviewed path. | Implemented; internal fault schedules and public load/query/receipt tests run natively. Native sync/I/O failure and healed receipt outcomes are exercised; durability remains unqualified. | Unavailable. |
| CLI argument capture | Native startup capture exercised. | Bounded `/proc/self/cmdline` capture and its unit tests run; CLI allocation and publication callers are implemented; retained evidence identifies exercised coverage. | Native argument capture is missing. |
| Complete regression gate | The complete gate runs here; retained evidence identifies its tested inputs. | All gate stages are implemented. Darwin-specific ACL recovery observations remain excluded; retained evidence identifies completed runs. | No complete gate available. |

For a Linux cross-compilation check, provision the `x86_64-unknown-linux-gnu`
target before offline use, then run:

```sh
RUSTFLAGS='-D warnings' cargo check --offline --locked --release --workspace --all-targets --target x86_64-unknown-linux-gnu
```

Compilation is not native execution. The [native Linux
baseline](../notes/evidence.md#platform-and-sanitizer-limitations) records the
tested revision, compiler image, discovered suites, and exclusions. Catalog,
legacy load/execution, and shared execution suites run on both native
implementations. Their stack observer uses Darwin's `pthread_get_stacksize_np`
or Linux's `pthread_getattr_np` and `pthread_attr_getstacksize`; an observation
failure fails the test.

The [stack contract](resources.md#native-paths-stack-and-io) defines the
48-KiB request and target-specific reported ceilings: 64 KiB on macOS and
144 KiB on GNU arm64. The Linux ceiling accounts for the native 128-KiB minimum
plus bounded runtime overhead; it is not a 64-KiB engine-frame claim.
Four public catalog, two legacy load/execution, and six internal scenarios run
with these checks. Their ordinary-thread counterparts preserve the same
functional expectations. No GNU arm64 stack scenario is ignored.

Run the focused scenarios with:

```sh
cargo test --offline --locked --release --workspace --all-targets stack -- --test-threads=1
```

The `stack` filter also selects a few scalar and ordinary-thread tests; inspect
the names and counts. Run the observer and its oversized-thread negative control
separately:

```sh
cargo test --offline --locked --release -p pipesql-filesystem --features test-stack-observation bounded_thread_observation
python3 tools/check-filesystem-abi.py
```

The native C control uses installed pthread headers, checks the current thread's
stack address/extent, and rejects an oversized extent. GNU arm64 additionally
requires native refusal of 48-KiB and 64-KiB requests. The Rust control verifies
that the actual scenario helper accepts a small thread and rejects a 2-MiB
thread. Both controls are included in the complete gate. Neither measures peak
engine frames, and neither substitutes for running the DBMS scenarios.

Platform-specific helpers compile with their actual consumers, without warning
suppression. Extending coverage requires executing the target contracts and
checking discovered, ignored, and failed tests.

The portability work still needs Windows path/handle/directory/locking/flush and
CLI implementations, native Linux/Windows CI, platform-specific failure
injection, and verified synchronization premises. The current macOS observations
do not transfer to another filesystem or operating system automatically.

Record the database filesystem as well as the OS. On the tested Docker
host-shared `fuseblk` mount, catalog creation intermittently observed different
path and open-file inode identities and correctly refused with
`RecoveryRequired`. That mount remains unqualified. Linux fixtures run on the
container's native filesystem; source exports may be mounted read-only, and
receipts can be copied out after the run. The [retained
counterexample](../notes/evidence.md#platform-and-sanitizer-limitations)
distinguishes this unresolved failure from passing native-filesystem checks.

### Diagnose filesystem identity

Use an unprivileged GNU/Linux shell with the pinned toolchain, C compiler, and
GNU `timeout`. In a Mac-hosted container, mount the source read-only and build
under `/tmp` on native container storage. Mount the host directory being tested
separately. Record the image digest, kernel, libc, user, and both mount types.

From the repository root, create a fresh native output directory and stock seed:

```sh
identity_work=$(mktemp -d /tmp/pipesql-identity.XXXXXX)
python3 -B tools/check-catalog-graph.py --seed-only --output "$identity_work/build"
cc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
  tools/fixtures/filesystem-identity.c -ldl -o "$identity_work/identity.so"
cc -std=c11 -Wall -Wextra -Werror tools/fixtures/filesystem-identity-control.c \
  -o "$identity_work/control"
```

Check the observer before interpreting its output. These commands use fresh
native directories. The stable control must exit zero without an identity line;
the replacement control must exit zero with exactly one identity line on stderr.
The final control deliberately omits the observer and must produce no such line.

```sh
timeout 30 env LD_PRELOAD="$identity_work/identity.so" \
  "$identity_work/control" "$identity_work/stable" stable
timeout 30 env LD_PRELOAD="$identity_work/identity.so" \
  "$identity_work/control" "$identity_work/replaced" replace
timeout 30 "$identity_work/control" "$identity_work/unobserved" replace
```

Run the unchanged caller on a new absolute database path on the mount under
investigation. Replace `/absolute/test-mount/new-database` before running:

```sh
timeout 30 "$identity_work/build/driver" /absolute/test-mount/new-database setup
```

Keep each exit status and failure output. Repeat on up to 20 fresh paths on each
mount; success does not clear an intermittent counterexample. Then repeat with
`env LD_PRELOAD="$identity_work/identity.so"` between `timeout 30` and the caller,
again using fresh paths. An identity line records the preceding pathname
`lstat`, descriptor `fstat64`, descriptor `statx`, and immediate pathname recheck.
It correlates same-thread root-file calls and adds read-only metadata queries.
It cannot distinguish concurrent replacement from filesystem identity changes
on its own. Exit 98 means observation failed, not that identities agreed.

The caller's `setup` operation checks stock completion, rows, and receipts; only
the seed step above also runs the independent graph inspection. Retain a compact
witness and environment description, then remove the owned output directory and
test databases. Do not delete an existing database or suppress a refusal to
complete this diagnostic.

### Windows implementation prerequisites

A Windows build needs native path, identity, directory, locking, byte-I/O, and
CLI owners before runtime qualification can start. Path conversion must preserve
admitted names and fallible allocation; the current Unix byte representation is
not a portable encoding contract. The test runner also needs descendant
ownership using Windows job objects before it can promise Unix-equivalent
cleanup.

Publication needs a documented mapping for each file flush, namespace
replacement, link, deletion, and directory barrier. Microsoft documents
[`FlushFileBuffers`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
for writable file handles and whole-volume flushing with administrative
privileges. Its [directory-handle
guide](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)
does not establish an equivalent unprivileged directory barrier.
[`ReplaceFileW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)
explicitly leaves `REPLACEFILE_WRITE_THROUGH` unsupported and documents failures
that can change names.
[`MoveFileExW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
describes write-through moves, including copy/delete flushing; that alone does
not specify every directory-barrier operation used by this protocol.

The engineering conclusion is that substituting similarly named calls is
insufficient. Establish the complete persistence mapping and failure outcomes,
then test real Windows handles, reparse points, sharing/deletion behavior,
resource refusal, and recovery on identified local filesystems. No Windows
runtime was available for this review, and no Windows implementation or
durability claim is made. The missing platform remains a product obligation.

## Build

```sh
cargo build --release --offline --locked
```

This produces the stock library and CLI under `target/release`. Follow [the
declared-table example](getting-started.md) to use the library. The
[example](../examples/declared.rs) is also compiled by the gate's all-target
Rust checks. Run its complete create/append/reopen/query flow with the command
in the walkthrough when changing that example or its APIs. Compilation alone
establishes neither runtime correctness nor release readiness.

## Documentation and tooling checks

```sh
python3 tools/check-maintenance.py
```

This checks current documentation links and anchors, maintained Python syntax,
source-manifest behavior, and native-caller build orchestration. It does not
build or execute the engine, native observers, or runtime campaigns. Use it
during non-engine maintenance; it is not a substitute for the complete gate
after a change that requires runtime verification.

Historical source captures and result packages are excluded from current-guide
link checks. Their original relative paths and input identities belong to their
recorded revision. Test fixtures exercise missing links and anchors, including wrapped link labels,
so a checker that silently ignores failures cannot pass its own tests.

## Focused verification

Choose the smallest check that can expose the changed contract. Examples:

```sh
cargo fmt --all --check
python3 tools/check-fixtures.py
python3 tools/test-q1-compare.py
cargo test --release --offline --locked --test catalog_lifecycle -- --test-threads=1
```

The fixture and comparator checks do not execute the engine. The Cargo test
command does. Check test counts and selected names; a successful empty selection
is not evidence. The [test map](../tests/README.md) and [tool
inventory](../tools/README.md) locate more specific checks and distinguish
public, internal, and stock-artifact paths.

## Qualify native sanitizer observations

Use this focused diagnostic when changing native mutex storage or investigating
sanitizer reports. It requires macOS or GNU/Linux, the pinned Rust toolchain,
a native compiler, and an installed nightly with AddressSanitizer support.
The [verified environment](../notes/evidence.md#platform-and-sanitizer-limitations)
identifies exercised compiler/runtime versions. Install a dated toolchain before
the offline diagnostic, or select an already installed equivalent:

```sh
rustup toolchain install nightly-2026-09-06 --profile minimal
python3 -B tools/check-native-sanitizer.py \
  --toolchain nightly-2026-09-06 --output /absolute/new-sanitizer-results
```

Supply a new directory outside the checkout. The command refuses existing outputs
and requires identical native targets for the stock and diagnostic compilers.
macOS uses `otool` to record linked libraries; GNU/Linux uses `ldd`. Run the command
as an unprivileged user. It does not install dependencies or change toolchains.

The command first checks a clean control and an isolated deliberate heap-bounds
fault. The fault must produce the expected AddressSanitizer report and exit 86.
It then executes all four native-mutex tests with the pinned compiler, the chosen
nightly without instrumentation, and the same nightly with AddressSanitizer.
Each run must report the exact required test names and successful completion.

Accept the result only when the command exits zero and `result.json` reports
`passed`, unchanged inputs, and no finalization errors. The directory retains
compiler/runtime identities, test-artifact hashes, separate stdout/stderr logs,
and before/after manifests. Build outputs are removed even after failure. A failed
control invalidates the observation; inspect its logs before interpreting any
production result. Remove the result directory after retaining necessary evidence.

The [sanitizer contract](verification.md#native-sanitizer-observation) explains
coverage and exclusions. This command does not replace the full regression gate
or establish race freedom, whole-engine memory safety, or durability.

## Inspect a persisted catalog

Use the independent inspector on a closed declared-table database or a quiescent
copy. The command takes the database lease without waiting and never repairs
files. Choose a new output path. The shell's noclobber option prevents
accidental overwrite:

```sh
(set -C; python3 tools/catalog_graph.py /absolute/database > /absolute/new-graph.json)
```

Require exit 0 before using the JSON. Exit 2 means invalid or unavailable input;
exit 3 means the diagnostic budget was exceeded. An unsuccessful command can
leave an empty output file, which the caller owns. The [inspection
contract](verification.md#independent-catalog-inspection) describes limits,
output fields, and interpretation. Unreferenced files are not automatically safe
to delete.

## Complete local gate

```sh
sh tools/check.sh
```

The shell command delegates to `python3 tools/check.py`. Both accept `--output
/absolute/new-result-directory`; without it, the gate creates and prints a
retained temporary result directory. Outputs must be outside the checkout and
must not already exist. Script paths may be absolute when invoking the gate from
another directory.

The gate is sequential and stops at a failing stage. It runs formatting,
maintenance, independent fixtures/models, warnings-denied release Clippy and
Rust documentation, Rust tests and doctests, and public semantic, allocation,
native, interruption, and persisted-graph campaigns. The stage list in
[`tools/check.py`](../tools/check.py) owns commands and deadlines. Each stage
reports elapsed time and its log path. Failed commands retain their exit status;
timeouts exit with status 124. Known regressions remain in the gate.

The result directory contains complete stage logs, before/after source
manifests, and `result.json` with scope, environment, commands, statuses, and
input integrity. Changed inputs or failed cleanup prevent a passing receipt. The
manifests cover engine, filesystem, vendor, configuration, tests, tools, and
documentation. They identify source bytes, not reproducible binaries. A Git
parent revision is recorded when available. Retain the exact checked commit or a
reconstructing patch for uncommitted inputs; runtime workloads need separate
input records.

The gate uses an isolated Cargo target and removes it after completion or
handled failure. Native callers also own separate temporary targets. Preserve
useful receipts and failing cases according to the engineering guide, then
remove old run directories. Parallel campaign execution still requires
shared-resource bounds and descendant-cleanup evidence; see the [campaign
isolation
decision](../notes/evidence.md#retired-implementations-and-gate-isolation).

The full campaign set runs on macOS and GNU/Linux. Run it as an unprivileged
user: root can bypass permission-refusal controls. Linux requires GNU
`/usr/bin/time`; macOS uses its native implementation. Linux initialization checks
root/component `lstat` and symlink `readlink` refusal, expanded pending suffixes,
and overlapping callers. It also rejects calls to libc `realpath`. Darwin checks
root metadata during its separate traversal.
The Linux run does not exercise Darwin data-mount spelling or the two Darwin ACL
recovery cells, and its target-specific stack ceiling remains explicit.

On GNU/Linux, `python3 tools/check-diagnostic-allocation.py --pathname-only`
sweeps allocation refusal during expanded-path creation and reopen. It checks
released allocations and successful retry, plus the common mutex control. This
focused command does not replace the complete allocation campaign.

On either platform, `python3
tools/check.py --scope core` runs the compiler, format, maintenance, ABI, model,
Rust-test, and documentation stages only. Its receipt explicitly says `core`; it
does not establish a passing full gate or qualify excluded platform tests.
Windows process ownership and native campaigns remain unfinished.

After the core gate, run the available native campaigns sequentially:

```sh
python3 tools/check-native-sync.py
python3 tools/check-native-io.py
python3 tools/check-catalog-interruption.py
```

These commands run on macOS and dynamically linked 64-bit GNU/Linux callers.
Linux uses `cc`, the glibc development headers, and libdl. Its observers are loaded
with `LD_PRELOAD`; Darwin uses `DYLD_INSERT_LIBRARIES`. Linux observer paths must
contain neither whitespace nor colons because the loader treats those as list
separators. The default temporary directories meet that requirement on the
reviewed hosts. Each campaign builds fresh artifacts and removes owned temporary
outputs; interruption's `--output` preserves a new result directory when requested.

Keep Linux database files on the identified native filesystem, not a host-shared
source mount. These campaigns test linked calls, error propagation, and process
termination with visible writes retained. They do not qualify static binaries,
other libc implementations, every Linux filesystem, or power-loss durability.

## Failures and interrupted work

Preserve the failing command, exit status, case identity, and logs. A filtered
passing run can diagnose a repair but cannot replace a failing or interrupted
complete gate. After interruption, check for owned descendant processes before
starting another campaign. Retain unresolved failures in the current plan.

Only rerun unchanged checks when a changed input, failure, or unresolved concern
justifies it. Documentation-only work needs fact, link, and example checks;
editorial changes do not need artificial runtime evidence. Changed executable
examples and test/tool behavior require verification appropriate to their actual
boundary, subject to the user's task constraints.

## Reading coverage

- [Local regression coverage](verification.md#local-regression-gate) specifies
  composition and failure obligations.
- [Public suite map](../tests/README.md) locates behavioral tests.
- [Tool inventory](../tools/README.md) locates independent and native callers.
- [Release criteria](verification.md#release-criteria) defines release qualification.

A local green gate does not establish whole-process memory bounds, every
operator composition, race freedom, stable formats, or power-loss certification.
Preserve the exclusions attached to each evidence package.
