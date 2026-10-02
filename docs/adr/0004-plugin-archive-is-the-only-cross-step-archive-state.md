# The plugin archive is the only state carried across the archive steps

Status: accepted

Step 3 (Create BA2 from Precombines) and Step 8 (Add Previs to Archive) both write `<plugin> - Main.ba2`, and Step 8 has to keep what Step 3 put there. Neither Archive2 nor BSArch can append to an archive in place. The batch works around that differently for each tool. Archive2 recovers the precombines *from the archive*: Step 8 extracts it into `Data`, waits 5s, deletes it and repacks `meshes\precombined,vis`. BSArch recovers them *from disk*: Step 3 moves the loose precombines into `<fo4>\BSArchTemp` and leaves them there, so Step 8 can move `vis` into the same tree and repack the whole thing. That leaves cross-step state outside the archive on the BSArch path, and the state is fragile. Step 1 clears `BSArchTemp`, but only on a fresh run (batch line 262). A `-bsarch` resume at Step 8 whose `BSArchTemp` is missing — deleted by hand, or Step 3 ran with Archive2 — packs `vis` alone over the archive and silently loses the precombines. Leftover staging from another plugin gets packed into the next one.

We make the plugin archive the only state either tool carries between the archive steps. Step 3 packs the precombines and removes the loose files, for both tools. Step 8 rebuilds the archive as its old contents plus `vis`. Archive2 keeps the batch's extract → 5s wait → repack exactly. BSArch unpacks the archive into a staging folder, moves `vis` in, waits 5s and packs. Both BSArch generations have `unpack` (0.9x, xEdit ≤ 4.1.5p, and 1.0, xEdit 4.1.5q); the batch simply never called it. So no BSArch version detection is needed, and a resume at Step 8 needs only the archive and `vis`.

Two rules follow from the same concern, which is that no failure may leave the user without their archive:

- **Build elsewhere, check, then swap.** Both tools write the new archive into a run-owned work folder beside `Data` (`<fo4>\ArchiveWork`) under its final name. The archive-exists check runs on that file, because `Archive2 -c` with no sources exits 0 and creates nothing. Only after that check passes is the old archive in `Data` replaced. The batch deletes the archive before repacking (440 → 442), so a failed repack there leaves loose files and no archive, a state the workflow cannot resume from cleanly.
- **Never delete what this run did not create.** A work folder left over at the start of Step 3 or 8 can only come from a crash, and it may hold the user's moved precombines. So it stops the run with its path named, and is not cleared. A failed move-back after a BSArch failure also stops the run, leaving staging in place. For the same reason, Step 1 does not port the `BSArchTemp` deletion at line 262; a batch-era `BSArchTemp` sits outside `Data` and is ignored.

Step 2 stops on zero meshes, and Step 3 stops when there are neither loose meshes nor an archive, so an archive missing at Step 8 means something outside the run deleted it. A `vis`-only archive is a broken build, because previs refers to the precombines. Step 8's `:ArchiveOnly` fallback (424, 445) is therefore dropped, and a missing archive at Step 8 stops the run. That also removes the BSArch `Meshes\vis\*.uvd` bug, which came from that fallback staging `vis` under `BSArchTemp\Meshes`. If the extract or unpack yields no precombined meshes, the run stops before the old archive is touched, instead of rebuilding from `vis` alone (441 → 445).

## Considered options

- **Batch parity: `BSArchTemp` carried from Step 3 to Step 8.** Rejected for the resume data-loss path above, and because it would need rules for what every resume point inherits from a folder the run cannot validate.
- **BSArch 1.0's multi-source pack, `old.ba2+Data\vis → new.ba2`.** No staging at all, but only xEdit 4.1.5q ships BSArch 1.0. Which BSArch a user has follows their xEdit, so 0.9x would still need one of the other designs, plus a version probe to choose between them.

## Consequences

BSArch Step 8 pays an extract it did not pay before, and BSArch `unpack` is new to this workflow. Its round-trip fidelity is a Windows experiment the Archive spec waits on. The batch's BSArch path has no MO2 wait. The port adds 5s before each BSArch pack, because under MO2 files moved out of the virtual `Data` may not have settled when BSArch reads the staging folder, which would produce an incomplete archive.
