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

**Workflow Plan**:
The ordered workflow steps for a Workflow Run, including which steps belong to the selected build mode, where resume starts, and which planned steps are currently runnable.
_Avoid_: Step list, runner capability, dry-run steps

**Workflow Operation**:
The domain behavior for one planned workflow step: its preconditions, external-tool action, postconditions, warnings, cleanup, and mode-specific rules. It is the step's build meaning before any specific process, filesystem, prompt, or timing adapter is chosen.
_Avoid_: Tool runner step, step helper, command wrapper

**Generate Precombines Operation**:
The Workflow Operation for Step 1, where a Workflow Run generates precombined meshes and prepares the precombine artifacts required by later steps.
_Avoid_: Step 1 checks, precombine helper, CK precombine wrapper
