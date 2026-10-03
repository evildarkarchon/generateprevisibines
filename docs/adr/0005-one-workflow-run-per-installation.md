# One Workflow Run per Fallout 4 installation

Status: accepted (2026-10-03)

Several rules assume that the state a run finds when it starts was left by an earlier run, not by one that is still going:

- ADR-0004 restores, sets aside or removes every `ArchiveWork` / `ArchiveWork.<n>` folder at the start of a run.
- `DllGuard` adopts a `*-PJMdisabled` DLL that has no original beside it (`docs/episodes.md` § *Creation Kit*, decided on #30).

Nothing makes that assumption true. Two invocations against the same installation would see each other's live state as crash leftovers. The second would move the first's staged precombines or `vis` back into `Data` while BSArch is packing them, swap in an archive the first is about to swap itself, or remove the folder the first is building in. A broken archive could then pass the archive-exists check. Beyond those rules, two runs would both write `CombinedObjects.esp`, `Previs.esp` and `Data\vis`, and run Creation Kit against the same `Data`. The batch has no guard against this either.

We allow one Workflow Run per installation, enforced by an OS file lock:

- **Acquire it as soon as the installation is known.** Once `WorkflowToolchainProbe::discover` has found `<fo4>`, and before Workflow Request Intake runs, the port opens `<fo4>\GeneratePrevisibines.lock` (creating it if needed) and takes an exclusive, non-blocking lock on it. Intake is covered too, because interactive intake can copy `xPrevisPatch.esp` into `Data`. A dry run never invokes the tools, so it takes no lock.
- **Stop at once if the lock is held.** The invocation stops with an error naming the installation and the lock file. It does not wait or retry. Two runs against one installation are a user mistake to report, not a queue to manage.
- **Hold it until the process ends**, through every step and Finish, and on every stop.
- **The lock is the OS lock, not the file.** The file is never deleted and never read. The OS releases the lock when the handle closes, which includes a crash, a kill or a power loss. So a lock file left behind is just a file, and the next run locks it normally. That is what lets ADR-0004 and the DLL adoption treat what they find as leftovers from a run that has ended.

The lock is `std::fs::File::try_lock` (stable since Rust 1.89), which is `LockFileEx` on Windows. It is safe std, so the main crate keeps `unsafe_code = "forbid"` (ADR-0003). It locks the file, not a name, so two spellings of the same installation (a different case, a junction, a mapped drive) still contend for one lock.

## Considered options

- **An ownership check per work folder** (a PID or a run ID written into each `ArchiveWork`). It would protect only the archive work folders. The DLL renames, the two working plugins and `Data\vis` would stay shared. A PID can also be reused after a crash, which would turn a real leftover into a "live" folder that is never recovered.
- **A PID file or a timestamped marker.** It needs a staleness heuristic, because a crash leaves the marker behind. A held OS lock needs none.
- **A named mutex** (`Global\…` keyed on the installation path). The OS releases it too, but it needs Win32 calls outside safe std, and a name built from a path string misses junctions and other aliases.
- **Waiting for the lock instead of stopping.** A queued run would start against whatever state the first run left. Stopping keeps the user in charge of the order.

## Consequences

- ADR-0004's start-of-run restore and the DLL guard's adoption can rely on what they find being leftovers. Neither needs an ownership check of its own.
- A crashed run leaves `GeneratePrevisibines.lock` in `<fo4>`. That is harmless. Windows may take a moment to release a dead process's lock, so a rerun started at once might stop with the in-use error and work on the next try.
- **Not covered:** `%TEMP%\Plugins.txt` is per user, not per installation, so runs against two different installations can still overwrite each other's FO4Edit plugin list. A per-run file name would fix that separately.
- **To confirm on a real machine:** that usvfs does not virtualize `<fo4>` itself when the run is launched through MO2, so that a run launched from MO2 and one launched outside it see the same lock file. ADR-0004 already assumes this for `<fo4>\ArchiveWork`.
