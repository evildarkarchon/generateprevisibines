# Workflow Operation seam owns step domain flow

Status: accepted

GeneratePrevisibines will use a Workflow Operation module as the execution seam for all 8 planned workflow steps. Each Workflow Operation owns the step's preconditions, external-tool action selection, postconditions, warnings, cleanup, and mode-specific rules, while Creation Kit, FO4Edit, archive, prompt, and wait behavior sit behind named adapters.

This replaces `ToolRunner` as the main execution seam because `ToolRunner` made the interface nearly match the implementation: callers selected a step and handed over split run state, while step meaning leaked across runner, checks, and tool modules. We rejected tool-specific ownership because CK, xEdit, BA2, and MO2 workarounds are necessary but are not themselves the workflow domain; putting success criteria in tool adapters would spread Workflow Run knowledge across those adapters.
