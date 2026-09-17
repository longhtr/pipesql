# Handle failure and recovery

A report, an import, and an export have different success conditions. A report
must finish producing rows. An import must publish one complete transaction.
An export must finish the query and deliver its encoded output. Use the returned
outcome to decide the next action; visible bytes alone are insufficient.

## Resolve a write

Save the transaction token before input processing proceeds. CSV import reports
it before reading CSV. Parquet import first validates the file's header and footer,
then reports it before reading pages. Both can refuse admission before any token
exists. The CLI writes and flushes the token before continuing.

If the process stops after that point, reopen and resolve the saved token before
retrying. The CLI command is:

```text
pipesql resolve --database ABSOLUTE_PATH --transaction TOKEN
  --memory-limit-bytes N --temp-limit-bytes N
```

Opening performs recovery; the subsequent lookup does not modify storage.
Require successful exit and `status=resolved`.

| Result | Action |
| --- | --- |
| `resolution=durable`, with a generation | The attempt committed. Do not append it again. |
| `resolution=aborted` | The issued attempt did not commit. A new attempt may retry the input. |
| Unknown or foreign token | Check the saved token and database. This is not an abort result. |
| Recovery failure | Keep the files and diagnose the refusal. Another import cannot settle authority. |

Tokens use 48 hexadecimal digits without a prefix or whitespace. Their database
identity and attempt number must both be nonzero. Issued numbers are never reused;
outcomes do not expire. A token beyond the issued prefix or from another database
returns `NotFound`. An active attempt returns `Contention`; unsettled or unavailable
state returns `RecoveryRequired`. The library's `resolve_commit` never repairs.

A successful commit can be followed by failure to print its receipt. The absence
of a success line therefore cannot justify retrying. This is why the earlier token
matters.

## Keep the original failure distinct from cleanup

Malformed input, exceeded import limits, cancellation, or input I/O failure aborts
the whole private import. A valid prefix is never committed. If removing private
work also fails, the cleanup requirement remains alongside the original cause.

| Library outcome | Interpretation |
| --- | --- |
| `Commit` | Publication and required synchronization finished. |
| Error followed by successful abort cleanup | The attempt did not commit; private removal was synchronized. |
| `CommitAmbiguous` | Publication may have happened. Close, reopen, and resolve. |
| `CleanupRequired` or `RecoveryRequired` | The handle cannot establish settled state. Close and reopen. |

Uncertain publication and failed cleanup retain temporary reservations while the
handle is alive. Closing frees accounts, but only reopening can settle the files.
Do not manually delete objects to make a refused open succeed: the surviving
records may be the evidence needed to distinguish committed data from private work.

Cancellation is cooperative. It is checked between engine steps and cannot stop
a blocked caller stream or syscall. After root replacement begins, publication
tries to finish without further cancellation checks. A cancellation request is
not proof that nothing committed.

## Publish an export only after success

Export success means the query reached `Finished`, the encoder wrote its final
record or footer, and the writer flushed. The CLI must also close the database
successfully before exit zero. A visible completion record may precede a failing
flush, and a byte limit may leave only part of a record.

For file output, write to a new private pathname. After success, publish that file
according to the application's replacement and synchronization requirements.
On failure, discard the incomplete file. PipeSQL releases execution resources but
does not remove caller output or flush pending buffers during drop.

Interrupted output writes return an error rather than being retried automatically.
Query errors retain their variants and SQL spans. [Formats](formats.md#result-completion)
describes the completion records and the bytes included in output limits.

## Choose storage that meets the engine's assumptions

GNU/Linux is primary; arm64 Linux and arm64 macOS have runtime checks. Windows is
deferred. Use local storage with consistent pathname and descriptor identities.
PipeSQL refuses an identity mismatch instead of retrying until observations agree.

The tested Docker Desktop VirtioFS shared mount sometimes returned inconsistent
inode identities, including outside PipeSQL. That issue remains unresolved.
Keep live databases on container-native storage when using the Linux runner;
shared source and build caches are separate. To investigate a mount, follow the
[filesystem check](../DEVELOPMENT.md#check-a-filesystem). A bounded successful run
cannot clear an intermittent counterexample.

Synchronization depends on the complete storage stack. macOS requests
`F_FULLFSYNC`; Linux requests its native file and directory synchronization.
A virtual disk's host policy still controls what those requests accomplish.
Apple distinguishes [fsync](https://developer.apple.com/documentation/virtualization/vzdiskimagesynchronizationmode/fsync)
and [full synchronization](https://developer.apple.com/documentation/virtualization/vzdiskimagesynchronizationmode/full)
policies; Linux documents the separate need to
[synchronize directory entries](https://man7.org/linux/man-pages/man2/fsync.2.html).

The tested Docker setup provided weaker host synchronization than native macOS.
Using container-native storage avoids the observed sharing-identity problem but
does not strengthen that host policy. Current container runs do not inherit the
former full-sync VM's qualification. Stronger durability qualification is deferred.
Process interruption and constructed corrupt files are not controlled power-loss
or torn-device-write tests.

Allocation checks cover particular workloads and allocator histories, not arbitrary
process-memory bounds. General concurrency and broader sanitizer coverage remain
incomplete, including runtime-library instrumentation. GNU arm64 pthreads also
require at least a 128-KiB stack plus runtime needs; a smaller requested stack is
not evidence of a smaller native bound. [Development](../DEVELOPMENT.md) owns the
check commands and their interpretation.
