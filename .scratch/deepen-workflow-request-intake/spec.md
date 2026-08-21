# Deepen Workflow Request Intake

Status: ready-for-agent

## Problem Statement

Workflow Request Intake appears to own the pre-run decision flow, but plugin readiness is currently delegated through a prompt adapter that also checks files, rejects archives, copies the seed plugin, waits for Mod Organizer 2 visibility, and decides whether intake can continue. As a result, the Intake tests replace the most important readiness behavior with a scripted answer instead of exercising the behavior that production uses.

This split makes readiness failures harder to trust and change. Archive precedence, interactive versus non-interactive behavior, seed-copy ordering, the mandatory visibility wait, resume decisions, and Workflow Run preparation are understood and tested across several modules rather than through the Workflow Request Intake interface described by the domain glossary.

## Solution

Make Workflow Request Intake a deep module that owns the complete decision flow from parsed choices and an already-discovered Workflow Toolchain Probe to either a prepared Workflow Run or a deliberate exit. Terminal prompts, filesystem access, and waits remain substitutable internal seams, but their adapters only perform their named roles; they do not decide readiness policy.

Production callers will use one concrete Intake module and one resolution operation. Tests will use the same resolution interface with recording prompt and wait adapters plus an in-memory file adapter. Seed-plugin copies will be byte-safe, and the required copy, visibility check, conditional five-second wait, and second visibility check will remain observable without using the real filesystem or actually sleeping.

## User Stories

1. As a Fallout 4 mod author, I want plugin readiness checked before a Workflow Run is prepared, so that invalid build attempts stop with an actionable error.
2. As a Fallout 4 mod author, I want an existing plugin archive rejected before other plugin readiness work, so that the current failure precedence remains predictable.
3. As an interactive user, I want to enter a plugin identity and receive the same validation behavior as today, so that this architecture change does not alter my workflow.
4. As an interactive user, I want an empty plugin-name response to exit deliberately, so that I can cancel Intake without receiving a failure.
5. As an interactive user, I want a missing target plugin to offer the seed-plugin copy choice, so that I can start from `xPrevisPatch.esp`.
6. As an interactive user, I want a missing seed plugin reported before I am asked whether to copy it, so that Intake never offers an impossible action.
7. As an interactive user, I want declining the seed copy to return a deliberate exit, so that refusal is not presented as an operational failure.
8. As an interactive user, I want accepting the seed copy to preserve every byte of the plugin, so that a binary `.esp` file is never treated as text.
9. As an interactive user running through Mod Organizer 2, I want Intake to wait five seconds only when the copied plugin is not immediately visible, so that the required VFS workaround is preserved without unnecessary delay.
10. As an interactive user, I want Intake to check visibility again after the wait, so that delayed VFS synchronization can complete successfully.
11. As an interactive user, I want Intake to report seed-copy failure when the target remains invisible after the wait, so that it never prepares a run against a missing plugin.
12. As an interactive user, I want an existing plugin to offer Continue, Exit, or Choose resume step, so that the current recovery workflow remains available.
13. As an interactive user, I want Continue to preserve my current resume intent, so that Intake does not silently restart from a different step.
14. As an interactive user, I want Exit from the existing-plugin prompt to be a deliberate exit, so that cancellation is distinct from an error.
15. As an interactive user, I want Choose resume step to update the Workflow Request, so that the prepared Workflow Run begins at my selected step.
16. As an interactive user, I want choosing to re-enter the plugin name to clear inherited resume intent, so that a newly selected plugin does not accidentally reuse the previous resume choice.
17. As an interactive user, I want invalid terminal input to be re-prompted by the terminal adapter, so that terminal parsing does not leak into readiness policy.
18. As a non-interactive user, I want an existing plugin to continue without prompts, so that unattended execution never blocks on terminal input.
19. As a non-interactive user, I want a missing plugin to fail without checking the seed or asking a question, so that unattended execution remains deterministic.
20. As a non-interactive user, I want supplied resume intent preserved, so that automation can restart at the requested workflow step.
21. As a user, I want Workflow Toolchain discovery to remain outside Intake, so that this change does not combine discovery policy with readiness policy.
22. As a user, I want full Workflow Toolchain preparation to begin only after plugin readiness succeeds, so that failed or cancelled Intake does not perform unnecessary preparation.
23. As a user, I want session-log creation to begin only after readiness succeeds, so that a deliberate exit does not leave behind a misleading run log.
24. As a user, I want prompt, filesystem, wait, validation, toolchain, and Workflow Run preparation failures propagated without being hidden, so that the original cause remains visible.
25. As a user, I want no Workflow Operation to execute during Intake, so that pre-run decisions remain separate from build execution.
26. As a maintainer, I want Workflow Request Intake to own archive, seed-copy, visibility, and resume ordering, so that readiness changes have one locality.
27. As a maintainer, I want the prompt adapter to ask typed questions only, so that it cannot replace readiness policy with a canned result.
28. As a maintainer, I want incompatible prompt answers prevented by types, so that the Intake implementation does not need runtime answer-shape checks.
29. As a maintainer, I want terminal wording and parsing kept in the terminal adapter, so that presentation changes do not alter the Intake domain flow.
30. As a maintainer, I want Intake-specific terminal code located behind the Intake module, so that understanding Intake does not require navigating a broad interactive module.
31. As a maintainer, I want the Intake module to expose a concrete, lifetime-free type, so that callers do not learn adapter generics or borrow details.
32. As a maintainer, I want the production constructor to be named for production rather than interactivity, so that its name remains accurate for non-interactive requests.
33. As a maintainer, I want deliberate exit represented by a named outcome, so that the interface distinguishes it from absence or failure.
34. As a maintainer, I want the existing boxed Workflow Run outcome retained, so that this refactor avoids unrelated enum-layout and lint changes.
35. As a maintainer, I want the executable directory to remain an explicit resolution input, so that Workflow Toolchain ownership is not redesigned in this feature.
36. As a maintainer, I want adapter ownership hidden in a plain private ports structure, so that the collection of adapters is data rather than another seam.
37. As a maintainer, I want the shallow Plugin Readiness carrier removed, so that callers no longer assemble or pass readiness data that Intake can derive itself.
38. As a maintainer, I want plugin, archive, and seed paths derived inside Intake from the candidate plugin and probed data directory, so that path conventions have one locality.
39. As a maintainer, I want the existing File Space seam reused for seed copying, so that a duplicate seed-only filesystem seam is not introduced.
40. As a maintainer, I want copied files represented as opaque bytes in the in-memory adapter, so that tests model binary plugins honestly.
41. As a maintainer, I want the existing Wait seam reused for the MO2 delay, so that production and recording adapters continue to exercise the same timing decision.
42. As a test author, I want every readiness behavior test to call the Intake resolution interface, so that tests and production cross the same seam.
43. As a test author, I want delayed visibility caused by the recording wait, so that tests prove the required copy, observe, wait, observe ordering rather than merely returning different values on consecutive checks.
44. As a test author, I want adapter-contract tests retained for terminal parsing, file copying, and waiting, so that each adapter remains trustworthy without duplicating Intake behavior tests.
45. As a test author, I want obsolete tests of private readiness helpers removed, so that the suite describes observable behavior rather than implementation structure.
46. As a future implementation agent, I want the current Rust behavior preserved exactly, so that architecture and batch-parity corrections can be reviewed independently.
47. As a future implementation agent, I want existing Workflow Operation and Creation Kit seams left unchanged, so that accepted architectural decisions are not re-litigated.
48. As a future implementation agent, I want the complete readiness scenario matrix specified, so that implementation can proceed without inventing policy.

## Implementation Decisions

- Workflow Request Intake becomes a concrete, lifetime-free deep module. Its public interface consists of a production constructor and one resolution operation.
- The production constructor is named `production`, because the same adapter set serves both interactive and non-interactive requests.
- Resolution continues to accept parsed command choices, the executable directory, and an already-discovered Workflow Toolchain Probe. Toolchain discovery remains outside Intake.
- Resolution returns the existing named Intake outcome: either a boxed ready Workflow Run or a deliberate exit. Errors continue through the repository's existing result and error types.
- The Intake implementation owns a private plain ports structure containing shared prompt, File Space, and Wait adapters. The ports structure is data and is not itself a seam or trait.
- Shared adapters use single-threaded reference-counted ownership internally. This keeps the public Intake type concrete while allowing tests to retain typed handles for assertions.
- A crate-private construction operation accepts the internal adapters. Production and tests then exercise the same resolution operation.
- The prompt seam becomes crate-private and exposes typed, domain-shaped questions for plugin identity, seed-copy confirmation, existing-plugin action, and resume-step selection.
- Prompt adapters own terminal display, parsing, invalid-input repetition, and conversion into typed answers. They do not inspect files, copy plugins, wait, construct Workflow Requests, or decide readiness.
- Intake validates every candidate plugin before readiness filesystem mutations, including candidates supplied by command-line parsing or recording prompt adapters.
- Intake-specific terminal implementation moves behind the Intake module. Workflow Operation prompts remain separate.
- Existing-plugin action types move inside the Intake module and become crate-private because Intake interprets them. Resume parsing types remain private to the terminal adapter. Seed-copy confirmation remains a boolean because it has only two valid answers.
- The generic public Intake type, public prompt trait, public terminal prompt adapter, public Plugin Readiness carrier, and the probe operation that constructs that carrier are removed from the supported interface.
- Intake derives target plugin, target archive, and seed-plugin paths privately from the candidate plugin identity and the Workflow Toolchain Probe's data directory.
- File Space gains one opaque binary-safe copy operation. It copies bytes without decoding, creates missing destination parents, replaces an existing destination, and reports filesystem failures.
- A successful copy does not guarantee immediate visibility through subsequent path observations. This preserves the Mod Organizer 2 VFS synchronization model.
- The system file adapter implements copy with the operating-system filesystem. The in-memory file adapter stores optional byte payloads instead of strings and performs an opaque byte-for-byte copy.
- Lossy text reads in the in-memory file adapter decode stored bytes only when text is explicitly requested. Existing contentless-file behavior remains representable.
- The existing Wait seam remains unchanged in the production interface. Its production and recording adapters continue to justify the seam.
- The recording wait adapter can perform a configured test effect. Delayed-visibility tests use that effect to reveal a pending copied file when the five-second wait occurs.
- Plugin readiness follows this fixed order: validate the candidate; derive readiness paths; reject an existing archive; inspect the target plugin; apply interactive or non-interactive missing-plugin policy; perform and verify any seed copy; handle existing-plugin choices; update or clear resume intent; then prepare the Workflow Run.
- An existing archive is reported before target-plugin existence or seed-copy behavior is considered.
- A missing non-interactive plugin fails without invoking any prompt, seed check, copy, or wait.
- Interactive seed-copy flow checks for the seed before asking, returns deliberate exit when declined, copies through File Space when accepted, observes target visibility immediately, waits exactly five seconds only when necessary, and observes once more.
- A copy failure propagates immediately without waiting. A target still invisible after the conditional wait produces the existing seed-copy failure.
- A copied target continues directly and is not subsequently treated as an initially existing plugin requiring another question.
- Existing non-interactive plugins continue without prompts. Existing interactive plugins preserve Continue, Exit, and Choose resume step behavior.
- Choosing to re-enter the plugin clears inherited resume intent before restarting plugin intake.
- Workflow Run preparation uses the same File Space adapter as readiness so session-log behavior remains observable through the Intake test surface.
- Workflow Run preparation and session-log creation occur only after readiness succeeds. Intake never executes a Workflow Operation.
- Current observable Rust behavior is preserved. Known batch-parity differences are not corrected as part of this architectural refactor.
- Added or substantially rewritten methods receive concise idiomatic documentation. New comments explain non-obvious ordering, conditional VFS waiting, opaque binary copying, and adapter ownership. Existing accurate comments are preserved.
- The existing domain glossary already assigns plugin readiness and resume intent to Workflow Request Intake, so this feature does not require a glossary change.
- The design follows the accepted Workflow Operation and internal-tool-seam decisions and does not require another architecture decision record.

## Testing Decisions

- Workflow Request Intake's resolution interface is the highest and primary test seam. Readiness behavior is tested only through the same resolution operation production calls.
- Readiness tests use a recording typed-prompt adapter, an in-memory File Space adapter, and a recording Wait adapter supplied through the private construction operation.
- Tests that expect a ready Workflow Run may use a real temporary minimal CKPE installation for the existing Workflow Toolchain preparation code. Readiness artifacts and session logs remain in the in-memory File Space.
- Replace tests that exercise the old whole-readiness prompt method or private request-resolution behavior. Do not retain those tests as a second behavioral test surface.
- Retain focused adapter-contract tests for terminal parsing and re-prompting, File Space operations, and Wait recording. These tests verify adapters, not Intake policy.
- Add File Space contract tests proving opaque binary bytes survive copying, destination parents are created, existing destinations are replaced, contentless files remain representable, and filesystem failures propagate.
- Preserve existing lossy-read tests while adapting them to byte-backed in-memory storage.
- Add Intake interface tests for existing archive precedence over target-plugin and seed behavior.
- Add an Intake interface test proving a missing non-interactive plugin returns the existing error without invoking prompt, copy, or wait adapters.
- Add an Intake interface test proving a missing seed fails before seed-copy confirmation.
- Add an Intake interface test proving declined seed copy returns deliberate exit and creates no session log.
- Add an Intake interface test proving immediate copy visibility produces a ready Workflow Run without waiting.
- Add an Intake interface test proving delayed copy visibility produces exactly one five-second wait and then a ready Workflow Run.
- Delayed-visibility setup keeps the copied bytes pending until the recording wait effect reveals them. This makes success depend on the required copy, observe, wait, observe order.
- Add an Intake interface test proving a target that remains invisible after the wait returns seed-copy failure.
- Add an Intake interface test proving a copy error propagates and no wait occurs.
- Add Intake interface tests for existing interactive plugin Continue, Exit, and Choose resume step outcomes.
- Add an Intake interface test proving existing non-interactive plugins continue without any prompt calls.
- Add an Intake interface test proving re-entering the plugin clears inherited resume intent before the next candidate is resolved.
- Add an Intake interface test proving Workflow Run preparation and session-log initialization happen only after readiness succeeds.
- Keep assertions on observable outcomes, recorded questions, filesystem state, wait durations, and prepared Workflow Run facts. Do not assert private helper calls or internal branching.
- Run formatting, the complete test suite, and pedantic linting after implementation.
- Update the repository knowledge graph after implementation as required by the project workflow.

## Out of Scope

- Implementing this specification as part of the specification-writing task.
- Moving Workflow Toolchain discovery into Workflow Request Intake.
- Moving executable-directory ownership into Workflow Toolchain Probe.
- Deepening Workflow Request validity or changing its public field model.
- Collapsing the raw discovery handoff or redesigning tool-path discovery.
- Deepening CKPE readiness or changing Workflow Toolchain validation ownership.
- Correcting existing batch-parity differences in plugin readiness behavior.
- Changing Workflow Operation, Operation Ports, Creation Kit, Process Runner, Wait, or Precombine Workspace architecture beyond reusing the established Wait seam.
- Designing FO4Edit or archive Episodes that do not yet have implemented Workflow Operation callers.
- Changing Workflow Plan ordering, runnability, resume membership, or operation registration.
- Executing any Workflow Operation during Intake.
- Reworking unrelated Workflow Operation terminal prompts.
- Adding a new domain glossary term or architecture decision record.

## Further Notes

- The design was derived from an architecture hot-spot review and a completed multi-round design tree. Every recommendation in that design tree was explicitly accepted.
- The selected design combines a concrete public Intake module with disciplined private adapters and typed prompt answers. A generic declarative question protocol was rejected because it permits incompatible answers and moves presentation strings into Intake.
- The repository test baseline was 154 passing tests when the architecture review was performed.
- The user explicitly requested that implementation not begin after the design was finalized. This spec is planning material marked ready for a future agent; publishing it does not authorize implementation in the current task.
