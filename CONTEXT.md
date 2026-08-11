# GeneratePrevisibines

GeneratePrevisibines automates the Fallout 4 precombine and previs build workflow while preserving the external-tool constraints inherited from the batch workflow.

## Language

**Workflow Request**:
The user's requested build: plugin identity, build mode, archive tool choice, resume step, and any Fallout 4 path override. It is intent before tool paths, CKPE configuration, logs, or runnable steps are resolved.
_Avoid_: ProjectConfig, CLI config, raw args

**Workflow Request Intake**:
The pre-run decision flow that turns a Workflow Request into either a Workflow Run or a deliberate user exit, including plugin readiness and resume intent. It is intake before any Workflow Operation runs.
_Avoid_: main orchestration, config builder, prompt flow

**Workflow Run**:
A prepared build attempt whose tools, CKPE configuration, logs, and executable workflow plan have been resolved and validated for the currently runnable steps.
_Avoid_: Session, context, engine run

**Workflow Toolchain**:
The external-tool readiness for a Workflow Run, including discovered tools, CKPE configuration, logs, and capability-specific validation for runnable Workflow Operations. It is readiness before any Workflow Operation chooses its step behavior.
_Avoid_: ToolPaths, ToolContext, discovery result

**Workflow Plan**:
The ordered workflow steps for a Workflow Run, including which steps belong to the selected build mode, where resume starts, and which planned steps are currently runnable. Runnability begins at the requested resume step and stops at the first unavailable Workflow Operation; it never skips an unavailable step to run a later one.
_Avoid_: Step list, runner capability, dry-run steps

**Workflow Operation**:
The domain behavior for one planned workflow step: its required toolchain readiness, preconditions, external-tool action, postconditions, warnings, cleanup, and mode-specific rules. It is the step's build meaning before any specific process, filesystem, prompt, or timing adapter is chosen. The adapters it needs arrive as Operation Ports — data handed to it per dispatch — rather than through a single adapter trait it is written against. Step identity, ordering, build-mode inclusion, and resume sequencing belong to the Workflow Plan rather than the Workflow Operation.
_Avoid_: Tool runner step, step helper, command wrapper

**Operation Ports**:
The adapters a Workflow Operation is handed for one dispatch: the Creation Kit episode, confirmations, and the file space. It is the set of substitutable things a Workflow Operation may reach, not a seam in its own right — process spawn and MO2 wait sit behind the Creation Kit episode, not beside it.
_Avoid_: adapter bundle, operation context, tool registry

**Generate Precombines Operation**:
The Workflow Operation for Step 1, where a Workflow Run generates precombined meshes and prepares the precombine artifacts required by later steps.
_Avoid_: Step 1 checks, precombine helper, CK precombine wrapper

**Precombine Workspace**:
The artifact space evaluated by the Generate Precombines Operation before and after Creation Kit runs, including existing precombined meshes and generated precombine outputs. It is the workspace being prepared and validated, not the Creation Kit command itself.
_Avoid_: Step 1 checks, precombine helper, artifact scanner
