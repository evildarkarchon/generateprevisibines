# Graph Report - generateprevisibines  (2026-08-03)

## Corpus Check
- 42 files · ~25,033 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 586 nodes · 1428 edges · 22 communities (20 shown, 2 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 14 edges (avg confidence: 0.87)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `739085a8`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- operations.rs
- GeneratePrevisibines
- toolchain.rs
- cli.rs
- run.rs
- files.rs
- BuildMode
- PluginIdentity
- precombine_workspace.rs
- CLAUDE.md
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
- .claude/CLAUDE.md

## God Nodes (most connected - your core abstractions)
1. `WorkflowStep` - 34 edges
2. `BuildMode` - 31 edges
3. `WorkflowRun` - 30 edges
4. `ProjectConfig` - 21 edges
5. `PluginIdentity` - 20 edges
6. `WorkflowToolchainProbe` - 20 edges
7. `Cli` - 19 edges
8. `project_config()` - 18 edges
9. `InMemoryFileSpace` - 17 edges
10. `ToolchainRequirements` - 17 edges

## Surprising Connections (you probably didn't know these)
- `Clean Build Mode` --conceptually_related_to--> `Workflow Plan`  [INFERRED]
  README.md → CONTEXT.md
- `Filtered Build Mode` --conceptually_related_to--> `Workflow Plan`  [INFERRED]
  README.md → CONTEXT.md
- `Xbox Build Mode` --conceptually_related_to--> `Workflow Plan`  [INFERRED]
  README.md → CONTEXT.md
- `Rust Scaffold` --conceptually_related_to--> `Workflow Vertical Slices`  [AMBIGUOUS]
  README.md → docs/future-features.md
- `GeneratePrevisibines Agent Guidance` --references--> `Triage Label Mapping`  [EXTRACTED]
  AGENTS.md → docs/agents/triage-labels.md

## Import Cycles
- 1-file cycle: `src/error.rs -> src/error.rs`
- 1-file cycle: `src/files.rs -> src/files.rs`
- 2-file cycle: `src/run.rs -> src/workflow/operations.rs -> src/run.rs`
- 2-file cycle: `src/tools/creation_kit.rs -> src/tools/mod.rs -> src/tools/creation_kit.rs`

## Hyperedges (group relationships)
- **Workflow Request to Operation Flow** — context_workflow_request, context_workflow_request_intake, context_workflow_run, context_workflow_toolchain, context_workflow_plan, context_workflow_operation [EXTRACTED 1.00]
- **Required External-Tool Workarounds** — docs_workarounds_fo4edit_keystroke_automation, docs_workarounds_mo2_timing_delays, docs_workarounds_dll_renaming_guard, docs_workarounds_archive2_extract_repack [EXTRACTED 1.00]

## Communities (22 total, 2 thin omitted)

### Community 0 - "operations.rs"
Cohesion: 0.08
Nodes (42): Cell, OperationExecution, WorkflowStep, ArtifactState, execute_registered_workflow(), prepare_production_workflow(), prepared_run(), production_operation_source() (+34 more)

### Community 1 - "GeneratePrevisibines"
Cohesion: 0.06
Nodes (59): External Tool Workaround Constraint, GeneratePrevisibines Agent Guidance, Repository Graphify Policy, GeneratePrevisibines Domain Language, Generate Precombines Operation, Precombine Workspace, Workflow Operation, Workflow Plan (+51 more)

### Community 2 - "toolchain.rs"
Cohesion: 0.07
Nodes (30): Default, CkpeInstallation, CreationKitToolchain, load_ckpe_installation(), parse_ckpe_log_file(), PluginReadiness, prepare_loads_ckpe_and_log_path_for_creation_kit(), prepare_requires_creation_kit_when_requested() (+22 more)

### Community 3 - "cli.rs"
Cohesion: 0.08
Nodes (41): From, I, OsStr, OsString, ArchiveToolFlags, attached_legacy_fo4_accepts_a_dash_prefixed_path(), attached_legacy_fo4_preserves_native_unix_path_bytes(), attached_legacy_fo4_preserves_native_windows_path_units() (+33 more)

### Community 4 - "run.rs"
Cohesion: 0.10
Nodes (30): prepare_creates_runnable_step_one_run(), prepare_preserves_filtered_and_xbox_partial_diagnostic_counts(), prepare_rejects_unavailable_explicit_resume_before_toolchain_readiness(), prepare_requires_creation_kit_for_registered_generate_precombines(), prepare_tolerates_non_utf8_ckpe_config_bytes(), prepared_run_executes_registered_plan_and_produces_precombine_artifacts(), ready_workflow_fixture(), ReadyWorkflowFixture (+22 more)

### Community 5 - "files.rs"
Cohesion: 0.12
Nodes (16): BTreeMap, Debug, Into, FileSpace, find_first_file_with_extension(), in_memory_space_answers_path_questions_and_mutates(), InMemoryFileSpace, Option (+8 more)

### Community 6 - "BuildMode"
Cohesion: 0.09
Nodes (30): Err, Fn, FromStr, BuildMode, Result, confirm_clear_precombined(), copy_seed_plugin(), ensure_plugin_ready() (+22 more)

### Community 7 - "PluginIdentity"
Cohesion: 0.14
Nodes (21): Box, P, PluginIdentity, String, cli_plugin_continue_yields_non_interactive_request(), interactive_resume_choice_updates_request(), interactive_resume_reprompt_clears_initial_resume_choice(), InteractiveWorkflowIntakePrompts (+13 more)

### Community 8 - "precombine_workspace.rs"
Cohesion: 0.18
Nodes (29): ck_log(), clear_precombined_meshes_empties_the_precombined_dir(), combined_objects(), detects_precombined_meshes_beneath_the_precombined_dir(), geometry_psg(), plugin_archive(), precombined_dir(), precombined_dir_is_the_meshes_precombined_layout_under_the_data_dir() (+21 more)

### Community 10 - "creation_kit.rs"
Cohesion: 0.12
Nodes (10): Command, Item, Iterator, CkOperation, CreationKitOps, operation_arg(), qualifier_args(), Result (+2 more)

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
Cohesion: 0.16
Nodes (8): ArchiveTool, ProjectConfig, Option, PathBuf, Self, ArchiveOps, Path, Result

### Community 20 - "Q: How are the Workflow Plan and dry-run preview built from registered Workflow Operations, including ordering, resume behavior, StepNotImplemented, and production operation availability?"
Cohesion: 0.40
Nodes (4): Answer, Outcome, Q: How are the Workflow Plan and dry-run preview built from registered Workflow Operations, including ordering, resume behavior, StepNotImplemented, and production operation availability?, Source Nodes

## Ambiguous Edges - Review These
- `Rust Scaffold` → `Workflow Vertical Slices`  [AMBIGUOUS]
  README.md · relation: conceptually_related_to

## Knowledge Gaps
- **17 isolated node(s):** `WorkflowRequestIntake<InteractiveWorkflowIntakePrompts>`, `graphify`, `graphify`, `Answer`, `Outcome` (+12 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **What is the exact relationship between `Rust Scaffold` and `Workflow Vertical Slices`?**
  _Edge tagged AMBIGUOUS (relation: conceptually_related_to) - confidence is low._
- **Why does `Error` connect `cli.rs` to `operations.rs`, `toolchain.rs`, `run.rs`, `BuildMode`, `PluginIdentity`, `precombine_workspace.rs`, `validation.rs`, `discovery.rs`?**
  _High betweenness centrality (0.087) - this node is a cross-community bridge._
- **Why does `BuildMode` connect `BuildMode` to `operations.rs`, `cli.rs`, `run.rs`, `PluginIdentity`, `precombine_workspace.rs`, `validation.rs`, `ProjectConfig`?**
  _High betweenness centrality (0.065) - this node is a cross-community bridge._
- **Why does `WorkflowStep` connect `operations.rs` to `cli.rs`, `run.rs`, `BuildMode`, `PluginIdentity`, `ProjectConfig`?**
  _High betweenness centrality (0.055) - this node is a cross-community bridge._
- **What connects `WorkflowRequestIntake<InteractiveWorkflowIntakePrompts>`, `graphify`, `graphify` to the rest of the system?**
  _17 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `operations.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.07645687645687646 - nodes in this community are weakly interconnected._
- **Should `GeneratePrevisibines` be split into smaller, more focused modules?**
  _Cohesion score 0.05902980713033314 - nodes in this community are weakly interconnected._