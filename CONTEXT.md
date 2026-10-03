# GeneratePrevisibines

GeneratePrevisibines automates the Fallout 4 precombine and previs build workflow while preserving the external-tool constraints inherited from the batch workflow.

## Language

**Workflow Request**:
The user's requested build: plugin identity, build mode, archive tool choice, resume step, and any Fallout 4 path override. It is intent before tool paths, CKPE configuration, logs, or runnable steps are resolved.
_Avoid_: ProjectConfig, CLI config, raw args

**Build Mode**:
The kind of build a Workflow Request asks for: Clean, Filtered, or Xbox. Clean and Xbox are both clean builds — they generate precombines from a clean cell set and build the CDX; they differ only in that Xbox keeps the geometry file uncompressed. Filtered generates filtered precombines and has no geometry or CDX steps. Build Mode decides Workflow Plan membership; no Build Mode compresses archives for Xbox.
_Avoid_: "filtered + Xbox compression" (the pre-V2.99 meaning), "clean mode" when any clean build is meant

**Workflow Request Intake**:
The pre-run decision flow that turns a Workflow Request into either a Workflow Run or a deliberate user exit, including plugin readiness and resume intent. It is intake before any Workflow Operation runs.
_Avoid_: main orchestration, config builder, prompt flow

**Workflow Run**:
A prepared build attempt whose tools, CKPE configuration, logs, and executable workflow plan have been resolved and validated for the currently runnable steps. At most one runs against a Fallout 4 installation at a time (ADR-0005), so anything a run finds left in the installation comes from a run that has ended.
_Avoid_: Session, context, engine run

**Workflow Toolchain**:
The external-tool readiness for a Workflow Run, including discovered tools, CKPE configuration, logs, and capability-specific validation for runnable Workflow Operations. It is readiness before any Workflow Operation chooses its step behavior.
_Avoid_: ToolPaths, ToolContext, discovery result

**Workflow Plan**:
The ordered workflow steps for a Workflow Run, including which steps belong to the selected build mode, where resume starts, and which planned steps are currently runnable. Runnability begins at the requested resume step and stops at the first unavailable Workflow Operation; it never skips an unavailable step to run a later one.
_Avoid_: Step list, runner capability, dry-run steps

**Workflow Operation**:
The domain behavior for one planned workflow step: its required toolchain readiness, preconditions, external-tool action, postconditions, warnings, cleanup, and mode-specific rules. It is the step's build meaning before any specific process, filesystem, prompt, or timing adapter is chosen. The adapters it needs arrive as Operation Ports — data handed to it per dispatch — rather than through a single adapter trait it is written against. Step identity, ordering, build-mode inclusion, and resume sequencing belong to the Workflow Plan rather than the Workflow Operation. A dispatch either completes — possibly having done nothing, possibly with Build Warnings — or stops the Workflow Run; there is no outcome that skips or jumps between planned steps, because build-mode gating belongs to the Workflow Plan.
_Avoid_: Tool runner step, step helper, command wrapper

**Build Warning**:
A condition a Workflow Operation or Finish reports while the Workflow Run continues — a tool finished but its log or exit status suggests a degraded result, a step found nothing to do where the batch warns, or Finish could not remove a Working File. It is surfaced when raised, on the console and in the session log. It is distinct from a run diagnostic, which is noticed while preparing a Workflow Run, before any Workflow Operation runs.
_Avoid_: diagnostic, soft error, non-fatal error

**Finish**:
The epilogue of a Workflow Run that runs only once every planned step has completed: it announces the build complete, lists the Patch Files, and offers to remove the Working Files. It is not a Workflow Operation — it has no step number, cannot be resumed to, and is not part of the Workflow Plan — and it never runs after a stop.
_Avoid_: step 9, cleanup step, `:Fin`

**Episode**:
One interaction between the build and something outside the process: a Creation Kit invocation, an FO4Edit script run, an archive operation, a user prompt, a wall-clock MO2 sync wait, a filesystem mutation, or a log read. It is the unit of external-tool behavior a Workflow Operation must reproduce, enumerated per step in `docs/episodes.md`; an episode bundles its own command line, delays, and log lifecycle, while the success criteria over its result stay with the Workflow Operation.
_Avoid_: tool call, adapter invocation, side effect

**Operation Ports**:
The adapters a Workflow Operation is handed for one dispatch: the Creation Kit episode, the FO4Edit episode, the archive episode, confirmations, and the file space. It is the set of substitutable things a Workflow Operation may reach, not a seam in its own right — process spawn, MO2 wait and FO4Edit's window handling sit behind the tool episodes, not beside them.
_Avoid_: adapter bundle, operation context, tool registry

**Generate Precombines Operation**:
The Workflow Operation for Step 1, where a Workflow Run generates precombined meshes and prepares the precombine artifacts required by later steps.
_Avoid_: Step 1 checks, precombine helper, CK precombine wrapper

**Precombine Workspace**:
The artifact space evaluated by the Generate Precombines Operation before and after Creation Kit runs, including existing precombined meshes and generated precombine outputs, which the Merge CombinedObjects step then requires before it launches FO4Edit. It is the workspace being prepared and validated, not the Creation Kit or FO4Edit command itself.
_Avoid_: Step 1 checks, precombine helper, artifact scanner

**Plugin Archive**:
The plugin's `- Main.ba2`. It holds the precombined meshes once Create BA2 from Precombines has run, and the previs as well once Add Previs to Archive has run. It is the only state the archive steps carry between them; Add Previs to Archive rebuilds it from its own contents plus the previs rather than adding to it.
_Avoid_: BA2 (as a synonym in prose), append (for the Add Previs rebuild), ArchiveWork contents

**Previs Workspace**:
The artifact space evaluated by the previs steps — the generated visibility files and the previs plugin — which the Generate Previs step clears and validates and the later previs merge and archive steps require. It is the workspace being prepared and validated, not the Creation Kit or FO4Edit command itself.
_Avoid_: vis checks, previs helper, uvd scanner

**Patch Files**:
The deliverables a completed build leaves in `Data` for the user to package or activate: the plugin and the Plugin Archive, plus — in clean builds — the CDX and the geometry file (compressed for Clean, uncompressed for Xbox). Finish lists them by Build Mode.
_Avoid_: outputs, artifacts, created files

**Working Files**:
The intermediate plugins `CombinedObjects.esp` and `Previs.esp`, which later steps merge into the plugin and which serve no purpose once the build completes. Finish offers to remove them, and always removes them when the run is non-interactive.
_Avoid_: temp files, scratch plugins
