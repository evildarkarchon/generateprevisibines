# GeneratePrevisibines

GeneratePrevisibines automates the Fallout 4 precombine and previs build workflow while preserving the external-tool constraints inherited from the batch workflow.

## Language

**Workflow Request**:
The user's requested build: plugin identity, build mode, archive tool choice, resume step, and any Fallout 4 path override. It is intent before tool paths, CKPE configuration, logs, or runnable steps are resolved.
_Avoid_: ProjectConfig, CLI config, raw args

**Workflow Run**:
A prepared build attempt whose tools, CKPE configuration, logs, and executable workflow plan have been resolved and validated for the currently runnable steps.
_Avoid_: Session, context, engine run

**Workflow Plan**:
The ordered workflow steps for a Workflow Run, including which steps belong to the selected build mode, where resume starts, and which planned steps are currently runnable.
_Avoid_: Step list, runner capability, dry-run steps
