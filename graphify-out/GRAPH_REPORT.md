# Graph Report - .  (2026-07-27)

## Corpus Check
- Corpus is ~33,275 words - fits in a single context window. You may not need a graph.

## Summary
- 580 nodes · 1389 edges · 18 communities
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 19 edges (avg confidence: 0.89)
- Token cost: 0 input · 0 output
- Token accounting note: Codex did not expose subagent usage metadata, so semantic-extraction token totals are unavailable; `cost.json` records zero rather than an estimate.

## Community Hubs (Navigation)
- Operation Execution State
- Project Policy and Domain
- CKPE Toolchain and Archives
- Legacy CLI Compatibility
- Project Path Configuration
- Graphify Tooling Workflow
- Build Mode Parsing
- Plugin Intake and Resume
- Precombine Workspace
- Workflow Steps and Capabilities
- Tool Execution and Timing
- CKPE Configuration Validation
- External Tool Discovery
- DLL Lifecycle Guard
- Session Logging
- Application Entry Point
- FO4Edit Adapter
- Lossy Text Reading

## God Nodes (most connected - your core abstractions)
1. `WorkflowStep` - 43 edges
2. `WorkflowRun` - 31 edges
3. `BuildMode` - 29 edges
4. `ProjectConfig` - 22 edges
5. `PluginIdentity` - 20 edges
6. `WorkflowToolchainProbe` - 20 edges
7. `Cli` - 19 edges
8. `prepared_run()` - 18 edges
9. `OperationCapability` - 17 edges
10. `workspace()` - 17 edges

## Surprising Connections (you probably didn't know these)
- `Domain Glossary Vocabulary Policy` --semantically_similar_to--> `Constrained Query Expansion`  [INFERRED] [semantically similar]
  docs/agents/domain.md → .codex/skills/graphify/references/query.md
- `CLAUDE.md Graphify Integration` --semantically_similar_to--> `Repository Graphify Policy`  [INFERRED] [semantically similar]
  .codex/skills/graphify/references/hooks.md → AGENTS.md
- `Clean Build Mode` --conceptually_related_to--> `Workflow Plan`  [INFERRED]
  README.md → CONTEXT.md
- `Filtered Build Mode` --conceptually_related_to--> `Workflow Plan`  [INFERRED]
  README.md → CONTEXT.md
- `Xbox Build Mode` --conceptually_related_to--> `Workflow Plan`  [INFERRED]
  README.md → CONTEXT.md

## Import Cycles
- 1-file cycle: `src/error.rs -> src/error.rs`
- 2-file cycle: `src/workflow.rs -> src/workflow/operations.rs -> src/workflow.rs`
- 2-file cycle: `src/run.rs -> src/workflow/operations.rs -> src/run.rs`
- 2-file cycle: `src/tools/creation_kit.rs -> src/tools/mod.rs -> src/tools/creation_kit.rs`
- 3-file cycle: `src/interactive.rs -> src/workflow.rs -> src/workflow/operations.rs -> src/interactive.rs`
- 3-file cycle: `src/run.rs -> src/workflow.rs -> src/workflow/operations.rs -> src/run.rs`

## Hyperedges (group relationships)
- **Graphify Extraction Pipeline** — _codex_skills_graphify_skill_structural_ast_extraction, _codex_skills_graphify_skill_semantic_extraction, _codex_skills_graphify_skill_graph_build_and_analysis [EXTRACTED 1.00]
- **Workflow Request to Operation Flow** — context_workflow_request, context_workflow_request_intake, context_workflow_run, context_workflow_toolchain, context_workflow_plan, context_workflow_operation [EXTRACTED 1.00]
- **Required External-Tool Workarounds** — docs_workarounds_fo4edit_keystroke_automation, docs_workarounds_mo2_timing_delays, docs_workarounds_dll_renaming_guard, docs_workarounds_archive2_extract_repack [EXTRACTED 1.00]

## Communities (18 total, 0 thin omitted)

### Community 0 - "Operation Execution State"
Cohesion: 0.07
Nodes (40): Cell, OperationExecution, ArtifactState, compatibility_requirements_delegate_to_production_source(), OperationAdapters, prepared_run(), production_capability_starts_with_step_one_only(), production_operation_source() (+32 more)

### Community 1 - "Project Policy and Domain"
Cohesion: 0.06
Nodes (60): CLAUDE.md Graphify Integration, External Tool Workaround Constraint, GeneratePrevisibines Agent Guidance, Repository Graphify Policy, GeneratePrevisibines Domain Language, Generate Precombines Operation, Precombine Workspace, Workflow Operation (+52 more)

### Community 2 - "CKPE Toolchain and Archives"
Cohesion: 0.08
Nodes (26): ArchiveTool, CkpeInstallation, CreationKitToolchain, load_ckpe_installation(), parse_ckpe_log_file(), PluginReadiness, prepare_loads_ckpe_and_log_path_for_creation_kit(), prepare_requires_creation_kit_when_requested() (+18 more)

### Community 3 - "Legacy CLI Compatibility"
Cohesion: 0.08
Nodes (42): From, I, OsStr, OsString, ArchiveToolFlags, attached_legacy_fo4_accepts_a_dash_prefixed_path(), attached_legacy_fo4_preserves_native_unix_path_bytes(), attached_legacy_fo4_preserves_native_windows_path_units() (+34 more)

### Community 4 - "Project Path Configuration"
Cohesion: 0.10
Nodes (24): ProjectConfig, PathBuf, capability_step_numbers(), execute_with_rejects_executor_capability_mismatch(), NoopAdapters, prepare_creates_runnable_step_one_run(), prepare_tolerates_non_utf8_ckpe_config_bytes(), request_to_project_config_uses_probe_data_dir() (+16 more)

### Community 5 - "Graphify Tooling Workflow"
Cohesion: 0.08
Nodes (42): Folder Watch, URL Ingestion, URL Ingestion and Folder Watch Reference, Extra Exports and Benchmark Reference, Neo4j and FalkorDB Exports, Graphify MCP Server, Token Reduction Benchmark, Wiki Export (+34 more)

### Community 6 - "Build Mode Parsing"
Cohesion: 0.09
Nodes (25): Err, FromStr, BuildMode, Option, Result, Self, confirm_clear_precombined(), copy_seed_plugin() (+17 more)

### Community 7 - "Plugin Intake and Resume"
Cohesion: 0.14
Nodes (22): Box, P, PluginIdentity, String, cli_plugin_continue_yields_non_interactive_request(), interactive_resume_choice_updates_request(), interactive_resume_reprompt_clears_initial_resume_choice(), InteractiveWorkflowIntakePrompts (+14 more)

### Community 8 - "Precombine Workspace"
Cohesion: 0.17
Nodes (25): ck_log_has_handle_array_error(), clear_precombined_meshes_removes_directory_when_present(), create_generated_outputs(), detects_precombined_meshes_recursively(), find_first_file_with_extension(), find_first_precombined_nif(), PrecombineWorkspace<'a>, prepare_ignores_nested_non_uvd_files_and_uvd_directories() (+17 more)

### Community 9 - "Workflow Steps and Capabilities"
Cohesion: 0.17
Nodes (12): WorkflowStep, clean_mode_includes_eight_steps(), filtered_mode_skips_psg_and_cdx(), OperationCapability, print_resume_menu(), resume_from_step_filters_earlier_steps(), Option, Result (+4 more)

### Community 10 - "Tool Execution and Timing"
Cohesion: 0.10
Nodes (14): Command, Default, Item, Iterator, CkOperation, CreationKitOps, operation_arg(), qualifier_args() (+6 more)

### Community 11 - "CKPE Configuration Validation"
Cohesion: 0.16
Nodes (16): CkpeConfigKind, ckpe_handle_limit_enabled(), clean_mode_rejects_spaces(), detect_ckpe_config_kind(), parse_ck_log_setting(), rejects_reserved_plugin_names(), Option, Path (+8 more)

### Community 12 - "External Tool Discovery"
Cohesion: 0.31
Nodes (13): clean_default_icon_path(), discover_fallout4_dir(), discover_fo4edit(), discover_tools(), finds_fo4edit_in_exe_dir(), fo4edit_from_registry(), format_version_line(), Option (+5 more)

### Community 13 - "DLL Lifecycle Guard"
Cohesion: 0.19
Nodes (10): Drop, RenamePair, DllGuard, renames_and_restores_on_drop(), restores_previous_renames_when_disable_fails(), Path, PathBuf, Result (+2 more)

### Community 14 - "Session Logging"
Cohesion: 0.26
Nodes (13): append_ck_log(), append_ck_log_tolerates_non_utf8_bytes(), append_log_line(), build_session_header(), init_session_log(), Path, PathBuf, Result (+5 more)

### Community 15 - "Application Entry Point"
Cohesion: 0.39
Nodes (8): ExitCode, emit_run_diagnostics(), emit_toolchain_diagnostic(), main(), print_banner(), Result, run(), run_dry_run()

### Community 16 - "FO4Edit Adapter"
Cohesion: 0.40
Nodes (4): Fo4EditOps, Option, Path, Result

### Community 17 - "Lossy Text Reading"
Cohesion: 0.60
Nodes (4): read_lossy(), Path, Result, String

## Ambiguous Edges - Review These
- `Rust Scaffold` → `Workflow Vertical Slices`  [AMBIGUOUS]
  README.md · relation: conceptually_related_to

## Knowledge Gaps
- **14 isolated node(s):** `WorkflowRequestIntake<InteractiveWorkflowIntakePrompts>`, `WorkflowOperationExecutor<ProductionOperationAdapters>`, `Token Reduction Benchmark`, `Semantic Hyperedges`, `Shortest Path Query` (+9 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **What is the exact relationship between `Rust Scaffold` and `Workflow Vertical Slices`?**
  _Edge tagged AMBIGUOUS (relation: conceptually_related_to) - confidence is low._
- **Why does `WorkflowStep` connect `Workflow Steps and Capabilities` to `Operation Execution State`, `Legacy CLI Compatibility`, `Project Path Configuration`, `Build Mode Parsing`, `Plugin Intake and Resume`?**
  _High betweenness centrality (0.085) - this node is a cross-community bridge._
- **Why does `Error` connect `Legacy CLI Compatibility` to `Operation Execution State`, `CKPE Toolchain and Archives`, `Project Path Configuration`, `Build Mode Parsing`, `Plugin Intake and Resume`, `Precombine Workspace`, `Workflow Steps and Capabilities`, `CKPE Configuration Validation`, `External Tool Discovery`?**
  _High betweenness centrality (0.075) - this node is a cross-community bridge._
- **Why does `BuildMode` connect `Build Mode Parsing` to `Operation Execution State`, `Legacy CLI Compatibility`, `Project Path Configuration`, `Plugin Intake and Resume`, `Precombine Workspace`, `Workflow Steps and Capabilities`, `CKPE Configuration Validation`?**
  _High betweenness centrality (0.058) - this node is a cross-community bridge._
- **What connects `WorkflowRequestIntake<InteractiveWorkflowIntakePrompts>`, `WorkflowOperationExecutor<ProductionOperationAdapters>`, `Token Reduction Benchmark` to the rest of the system?**
  _14 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Operation Execution State` be split into smaller, more focused modules?**
  _Cohesion score 0.07146087743102668 - nodes in this community are weakly interconnected._
- **Should `Project Policy and Domain` be split into smaller, more focused modules?**
  _Cohesion score 0.0576271186440678 - nodes in this community are weakly interconnected._
