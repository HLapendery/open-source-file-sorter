# Filesystem safety

## Transfer guarantees

Sortify moves regular files only and never enables overwrite. Before any
mutation it validates the source, selects a free destination according to the
collision policy, rejects destination-directory symbolic links, and writes a
durable journal intent.

On one filesystem the engine creates a hard link at the destination. This is an
atomic, no-clobber publication: if another process creates that name first, the
operation fails or follows the configured collision policy. The destination is
synchronized before the source name is removed.

Across filesystems the engine:

1. creates a unique private file in the destination directory with
   create-new semantics;
2. streams the source into it using a bounded buffer;
3. copies permissions and timestamps, then synchronizes the file;
4. compares source and temporary content byte-for-byte;
5. rechecks source identity, length, and modification time;
6. publishes with a no-clobber hard link inside the destination filesystem;
7. synchronizes the published file and best-effort synchronizes its directory;
8. journals `PUBLISHED`;
9. removes the private name and revalidates the source;
10. removes the source and journals `COMPLETED`.

At no point is an existing destination replaced. The source is not deleted
until verified content has been published and synchronized.

## Durable journal and recovery

SQLite runs in WAL mode with `synchronous=FULL`. The journal stores native path
bytes where the platform exposes them, the actual selected destination,
temporary path, transfer kind, source fingerprint, and explicit state.
A partial unique index prevents another process using the same normalized source
path while an operation is unfinished.

`sorter recover` only inspects. `sorter recover --apply` mutates state or files
when the actual filesystem proves a safe outcome:

- a journaled publication whose source still exists is compared before the
  source is removed;
- a verified private copy is compared again, published without clobbering, and
  only then allowed to replace the source name;
- a source with no destination or temporary file is retained and the abandoned
  operation is closed as failed;
- a journaled publication with only its destination remaining is marked
  complete after metadata validation.

If files differ, publication cannot be proven, a symlink appears, or paths are
missing ambiguously, recovery preserves everything and reports
`RECOVERY_REQUIRED`. Repeating recovery is idempotent because terminal
operations are no longer selected and every publication is no-clobber.

## Collision behavior

- `skip` retains the source and existing destination;
- `rename-with-suffix` selects `name (1).ext` and retries a pre-publication
  race without overwriting;
- `error` retains both paths and reports the collision.

Overwrite is intentionally unavailable.

## Symbolic links and paths

The scanner does not follow directory links and does not plan link entries.
Direct source links, source-root links, and existing destination-directory links
are rejected. Native path bytes are preserved in the recovery journal on Unix;
display fields and textual matching remain Unicode-oriented.

There is an unavoidable check/use race if another process replaces a validated
directory. Directory-handle-relative platform APIs may reduce this risk in a
future hardening pass.

## Durability and stabilization limits

`fsync` behavior depends on the operating system, local or network filesystem,
mount options, storage controller, and hardware. Some platforms and filesystems
do not support synchronizing directory metadata; this is logged as a warning.
No userspace application can promise persistence against every kernel,
filesystem, device, or power failure.

Source-change detection combines an open-file identity fingerprint, size,
modification time, and byte comparison. It detects normal concurrent writers
during a long transfer, but cannot lock out a hostile writer that deliberately
restores metadata inside the final validation-to-unlink interval.

Watch mode waits for two identical metadata observations and then requires size
and modification time to remain unchanged for the configured stabilization
delay. Events reset stabilization, bounded queue overflow requests
reconciliation, and common temporary filename patterns are excluded.

This remains a heuristic: a writer can pause longer than the delay and resume
later. Native event delivery can also be incomplete on network filesystems,
inside some virtualized Docker environments, or during event storms. Periodic
streaming reconciliation is therefore the source of eventual discovery.
One-shot and scheduled callers remain responsible for choosing execution times
when non-temporary files are not being actively written.

## Test coverage

Fault-injection tests interrupt before and during copy, after copy and
verification, before and after publication, before and after source deletion,
and while recording journal transitions. Tests assert that no interruption
silently loses the only complete copy. Recovery tests cover preview, safe
completion, verified temporary publication, ambiguity preservation, and
idempotent repetition.
