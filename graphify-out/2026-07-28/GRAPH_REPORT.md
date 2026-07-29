# Graph Report - generateprevisibines  (2026-07-28)

## Corpus Check
- 48 files · ~33,537 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 585 nodes · 1373 edges · 21 communities
- Extraction: 98% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 20 edges (avg confidence: 0.89)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `c9603df9`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- operations.rs
- GeneratePrevisibines
- toolchain.rs
- cli.rs
- WorkflowRun
- Graphify Pipeline
- interactive.rs
- BuildMode
- precombine_workspace.rs
- .steps_for
- creation_kit.rs
- validation.rs
- discovery.rs
- DllGuard
- logging.rs
- main.rs
- .run_script
- read_lossy
- Q: $improve-codebase-architecture
- ProjectConfig
- Q: How are the Workflow Plan and dry-run preview built from registered Workflow Operations, including ordering, resume behavior, StepNotImplemented, and production operation availability?

## God Nodes (most connected - your core abstractions)
1. `WorkflowStep` - 36 edges
2. `BuildMode` - 31 edges
3. `WorkflowRun` - 30 edges
4. `ProjectConfig` - 22 edges
5. `PluginIdentity` - 20 edges
6. `WorkflowToolchainProbe` - 20 edges
7. `Cli` - 19 edges
8. `prepared_run()` - 18 edges
9. `ToolchainRequirements` - 17 edges
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
- 2-file cycle: `src/run.rs -> src/workflow/operations.rs -> src/run.rs`
- 2-file cycle: `src/tools/creation_kit.rs -> src/tools/mod.rs -> src/tools/creation_kit.rs`

## Hyperedges (group relationships)
- **Graphify Extraction Pipeline** — _codex_skills_graphify_skill_structural_ast_extraction, _codex_skills_graphify_skill_semantic_extraction, _codex_skills_graphify_skill_graph_build_and_analysis [EXTRACTED 1.00]
- **Workflow Request to Operation Flow** — context_workflow_request, context_workflow_request_intake, context_workflow_run, context_workflow_toolchain, context_workflow_plan, context_workflow_operation [EXTRACTED 1.00]
- **Required External-Tool Workarounds** — docs_workarounds_fo4edit_keystroke_automation, docs_workarounds_mo2_timing_delays, docs_workarounds_dll_renaming_guard, docs_workarounds_archive2_extract_repack [EXTRACTED 1.00]

## Communities (21 total, 0 thin omitted)

### Community 0 - "operations.rs"
Cohesion: 0.07
Nodes (48): Cell, OperationExecution, WorkflowStep, ArtifactState, prepare_production_workflow(), prepared_run(), production_operation_source(), production_preparation_unions_requirements_for_the_runnable_plan() (+40 more)

### Community 1 - "GeneratePrevisibines"
Cohesion: 0.06
Nodes (58): External Tool Workaround Constraint, GeneratePrevisibines Agent Guidance, GeneratePrevisibines Domain Language, Generate Precombines Operation, Precombine Workspace, Workflow Operation, Workflow Plan, Workflow Request (+50 more)

### Community 2 - "toolchain.rs"
Cohesion: 0.08
Nodes (28): ArchiveTool, PluginIdentity, String, CkpeInstallation, CreationKitToolchain, load_ckpe_installation(), parse_ckpe_log_file(), PluginReadiness (+20 more)

### Community 3 - "cli.rs"
Cohesion: 0.08
Nodes (41): From, I, OsStr, OsString, ArchiveToolFlags, attached_legacy_fo4_accepts_a_dash_prefixed_path(), attached_legacy_fo4_preserves_native_unix_path_bytes(), attached_legacy_fo4_preserves_native_windows_path_units() (+33 more)

### Community 4 - "WorkflowRun"
Cohesion: 0.12
Nodes (27): prepare_creates_runnable_step_one_run(), prepare_preserves_filtered_and_xbox_partial_diagnostic_counts(), prepare_rejects_unavailable_explicit_resume_before_toolchain_readiness(), prepare_requires_creation_kit_for_registered_generate_precombines(), prepare_tolerates_non_utf8_ckpe_config_bytes(), ready_workflow_fixture(), ReadyWorkflowFixture, request_to_project_config_uses_probe_data_dir() (+19 more)

### Community 5 - "Graphify Pipeline"
Cohesion: 0.07
Nodes (44): Folder Watch, URL Ingestion, URL Ingestion and Folder Watch Reference, Extra Exports and Benchmark Reference, Neo4j and FalkorDB Exports, Graphify MCP Server, Token Reduction Benchmark, Wiki Export (+36 more)

### Community 6 - "interactive.rs"
Cohesion: 0.20
Nodes (16): confirm_clear_precombined(), copy_seed_plugin(), ensure_plugin_ready(), ensure_plugin_ready_rejects_existing_archive(), ExistingPluginAction, parse_existing_plugin_choice(), parse_resume_step_choice(), prompt_existing_plugin_action() (+8 more)

### Community 7 - "BuildMode"
Cohesion: 0.11
Nodes (23): Box, Err, FromStr, P, BuildMode, Result, cli_plugin_continue_yields_non_interactive_request(), interactive_resume_choice_updates_request() (+15 more)

### Community 8 - "precombine_workspace.rs"
Cohesion: 0.17
Nodes (25): ck_log_has_handle_array_error(), clear_precombined_meshes_removes_directory_when_present(), create_generated_outputs(), detects_precombined_meshes_recursively(), find_first_file_with_extension(), find_first_precombined_nif(), PrecombineWorkspace<'a>, prepare_ignores_nested_non_uvd_files_and_uvd_directories() (+17 more)

### Community 9 - ".steps_for"
Cohesion: 0.21
Nodes (10): Fn, clean_mode_preserves_all_steps_in_canonical_order(), filtered_mode_preserves_its_canonical_membership_and_order(), print_resume_menu(), resume_preserves_the_canonical_suffix(), Option, Result, Self (+2 more)

### Community 10 - "creation_kit.rs"
Cohesion: 0.10
Nodes (14): Command, Default, Item, Iterator, CkOperation, CreationKitOps, operation_arg(), qualifier_args() (+6 more)

### Community 11 - "validation.rs"
Cohesion: 0.16
Nodes (16): CkpeConfigKind, ckpe_handle_limit_enabled(), clean_mode_rejects_spaces(), detect_ckpe_config_kind(), parse_ck_log_setting(), rejects_reserved_plugin_names(), Option, Path (+8 more)

### Community 12 - "discovery.rs"
Cohesion: 0.31
Nodes (13): clean_default_icon_path(), discover_fallout4_dir(), discover_fo4edit(), discover_tools(), finds_fo4edit_in_exe_dir(), fo4edit_from_registry(), format_version_line(), Option (+5 more)

### Community 13 - "DllGuard"
Cohesion: 0.19
Nodes (10): Drop, RenamePair, DllGuard, renames_and_restores_on_drop(), restores_previous_renames_when_disable_fails(), Path, PathBuf, Result (+2 more)

### Community 14 - "logging.rs"
Cohesion: 0.26
Nodes (13): append_ck_log(), append_ck_log_tolerates_non_utf8_bytes(), append_log_line(), build_session_header(), init_session_log(), Path, PathBuf, Result (+5 more)

### Community 15 - "main.rs"
Cohesion: 0.39
Nodes (8): ExitCode, emit_run_diagnostics(), emit_toolchain_diagnostic(), main(), print_banner(), Result, run(), run_dry_run()

### Community 16 - ".run_script"
Cohesion: 0.40
Nodes (4): Fo4EditOps, Option, Path, Result

### Community 17 - "read_lossy"
Cohesion: 0.60
Nodes (4): read_lossy(), Path, Result, String

### Community 18 - "Q: $improve-codebase-architecture"
Cohesion: 0.40
Nodes (4): Answer, Outcome, Q: $improve-codebase-architecture, Source Nodes

### Community 19 - "ProjectConfig"
Cohesion: 0.17
Nodes (7): ProjectConfig, Option, PathBuf, Self, ArchiveOps, Path, Result

### Community 20 - "Q: How are the Workflow Plan and dry-run preview built from registered Workflow Operations, including ordering, resume behavior, StepNotImplemented, and production operation availability?"
Cohesion: 0.40
Nodes (4): Answer, Outcome, Q: How are the Workflow Plan and dry-run preview built from registered Workflow Operations, including ordering, resume behavior, StepNotImplemented, and production operation availability?, Source Nodes

## Ambiguous Edges - Review These
- `Rust Scaffold` → `Workflow Vertical Slices`  [AMBIGUOUS]
  README.md · relation: conceptually_related_to

## Knowledge Gaps
- **20 isolated node(s):** `WorkflowRequestIntake<InteractiveWorkflowIntakePrompts>`, `WorkflowOperationExecutor<ProductionOperationAdapters>`, `Answer`, `Outcome`, `Source Nodes` (+15 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **What is the exact relationship between `Rust Scaffold` and `Workflow Vertical Slices`?**
  _Edge tagged AMBIGUOUS (relation: conceptually_related_to) - confidence is low._
- **Why does `Error` connect `cli.rs` to `operations.rs`, `toolchain.rs`, `WorkflowRun`, `interactive.rs`, `BuildMode`, `precombine_workspace.rs`, `.steps_for`, `validation.rs`, `discovery.rs`?**
  _High betweenness centrality (0.073) - this node is a cross-community bridge._
- **Why does `BuildMode` connect `BuildMode` to `operations.rs`, `cli.rs`, `WorkflowRun`, `interactive.rs`, `precombine_workspace.rs`, `.steps_for`, `validation.rs`, `ProjectConfig`?**
  _High betweenness centrality (0.062) - this node is a cross-community bridge._
- **Why does `WorkflowStep` connect `operations.rs` to `cli.rs`, `WorkflowRun`, `interactive.rs`, `BuildMode`, `.steps_for`, `ProjectConfig`?**
  _High betweenness centrality (0.059) - this node is a cross-community bridge._
- **What connects `WorkflowRequestIntake<InteractiveWorkflowIntakePrompts>`, `WorkflowOperationExecutor<ProductionOperationAdapters>`, `Answer` to the rest of the system?**
  _20 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `operations.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.06660006660006661 - nodes in this community are weakly interconnected._
- **Should `GeneratePrevisibines` be split into smaller, more focused modules?**
  _Cohesion score 0.060496067755595885 - nodes in this community are weakly interconnected._