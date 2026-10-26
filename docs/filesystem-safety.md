# Filesystem safety

## Phase 2 guarantees

For a regular file on one filesystem:

1. validate source metadata and destination policy;
2. reject source and destination-directory symbolic links;
3. create destination directories;
4. atomically publish a hard link without replacing any existing name;
5. synchronize the destination file and best-effort synchronize its directory;
6. remove the source name;
7. best-effort synchronize the source directory.

The source is never deleted before the destination refers to the same inode and
the destination file has synchronized successfully. Collision races are handled
by the atomic hard-link operation, not only by a prior existence check.

If removing the source fails, both names are preserved and the operation is
marked `RECOVERY_REQUIRED`. Repeating a suffix operation may choose a new suffix,
so automated recovery is not claimed yet.

## Symbolic links

The scanner does not follow directory links and does not plan link entries.
Direct source links, source-root links, and existing destination-directory links
are rejected. There is still an unavoidable check/use race when another process
replaces a validated directory; Phase 3 should use directory-handle-relative
platform APIs where available.

## Durability limitations

`fsync` behavior depends on the operating system, local or network filesystem,
mount options, storage controller, and hardware. Some platforms/filesystems do
not support synchronizing directory metadata; this is logged as a warning.
No application can promise persistence against every hardware or kernel failure.

## Not implemented yet

Cross-filesystem operations fail without modifying the source. Phase 3 will use
a temporary destination, streaming copy, synchronization, verification,
no-clobber publication, source-change detection, and only then source removal.
It will include fault injection and actual-filesystem-state recovery.

The filesystem APIs operate on native paths and can move non-UTF-8 names on Unix.
Current glob/regex matching and SQLite display fields use lossy Unicode
conversion; exact byte-oriented rule matching and lossless path persistence are
future work.

