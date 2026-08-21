//! Workflow Request Intake.
//!
//! This module owns the pre-run decision flow from parsed command-line choices
//! to either a prepared Workflow Run or a deliberate user exit.

use std::path::Path;
use std::rc::Rc;

use crate::cli::Cli;
use crate::config::{BuildMode, PluginIdentity, WorkflowStep};
use crate::error::{Error, Result};
use crate::files::{FileSpace, SystemFileSpace};
use crate::interactive::{self, ExistingPluginAction};
use crate::run::{WorkflowRequest, WorkflowRun};
use crate::toolchain::{PluginReadiness, WorkflowToolchainProbe};

/// Result of Workflow Request Intake.
#[derive(Debug, Clone)]
pub enum WorkflowIntakeOutcome {
    /// A Workflow Run is prepared and ready to execute.
    Ready(Box<WorkflowRun>),
    /// The user deliberately exited during intake prompts.
    Exited,
}

/// Prompt adapter used by Workflow Request Intake.
pub trait WorkflowIntakePrompts {
    /// Prompt for the plugin identity, or return `None` when the user exits.
    fn prompt_plugin_name(&self, build_mode: BuildMode) -> Result<Option<PluginIdentity>>;

    /// Ensure the plugin is ready before full Workflow Run preparation.
    fn ensure_plugin_ready(&self, readiness: &PluginReadiness) -> Result<ExistingPluginAction>;

    /// Prompt for the Workflow Step to resume from, or return `None` to re-prompt plugin intake.
    fn prompt_resume_step(&self, build_mode: BuildMode) -> Result<Option<WorkflowStep>>;
}

/// Terminal-backed prompt adapter for production intake.
#[derive(Debug, Default, Clone, Copy)]
pub struct InteractiveWorkflowIntakePrompts;

impl WorkflowIntakePrompts for InteractiveWorkflowIntakePrompts {
    fn prompt_plugin_name(&self, build_mode: BuildMode) -> Result<Option<PluginIdentity>> {
        interactive::prompt_plugin_name(build_mode)
    }

    fn ensure_plugin_ready(&self, readiness: &PluginReadiness) -> Result<ExistingPluginAction> {
        interactive::ensure_plugin_ready(readiness)
    }

    fn prompt_resume_step(&self, build_mode: BuildMode) -> Result<Option<WorkflowStep>> {
        interactive::prompt_resume_step(build_mode)
    }
}

/// Private shared ports owned by Workflow Request Intake.
#[derive(Clone)]
struct IntakePorts {
    // Shared ownership keeps the supported Intake type lifetime-free while tests retain their
    // concrete recording handles for assertions after resolution.
    prompts: Rc<dyn WorkflowIntakePrompts>,
    files: Rc<dyn FileSpace>,
}

impl std::fmt::Debug for IntakePorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntakePorts")
            .field("prompts", &"shared WorkflowIntakePrompts")
            .field("files", &self.files)
            .finish()
    }
}

/// Turns Workflow Requests into ready Workflow Runs without exposing adapter types or lifetimes.
#[derive(Debug, Clone)]
pub struct WorkflowRequestIntake {
    ports: IntakePorts,
}

impl WorkflowRequestIntake {
    /// Create production Workflow Request Intake backed by terminal prompts and the system files.
    #[must_use]
    pub fn production() -> Self {
        Self::new(
            Rc::new(InteractiveWorkflowIntakePrompts),
            Rc::new(SystemFileSpace),
        )
    }

    /// Create Intake from owned shared adapters while allowing tests to retain typed handles.
    #[must_use]
    pub(crate) fn new<P, F>(prompts: Rc<P>, files: Rc<F>) -> Self
    where
        P: WorkflowIntakePrompts + 'static,
        F: FileSpace + 'static,
    {
        Self {
            ports: IntakePorts { prompts, files },
        }
    }

    /// Resolve parsed choices and probed toolchain facts into a Workflow Run or a deliberate exit.
    pub fn resolve(
        &self,
        cli: &Cli,
        exe_dir: &Path,
        probe: &WorkflowToolchainProbe,
    ) -> Result<WorkflowIntakeOutcome> {
        let Some(request) = self.resolve_request(cli, probe)? else {
            return Ok(WorkflowIntakeOutcome::Exited);
        };

        // Reusing the readiness File Space keeps log creation after successful readiness and
        // lets callers observe both phases through one adapter rather than a hidden second one.
        let run = WorkflowRun::prepare(&request, exe_dir, probe, self.ports.files.as_ref())?;
        Ok(WorkflowIntakeOutcome::Ready(Box::new(run)))
    }

    /// Resolve command choices into a request after candidate readiness succeeds.
    fn resolve_request(
        &self,
        cli: &Cli,
        probe: &WorkflowToolchainProbe,
    ) -> Result<Option<WorkflowRequest>> {
        if let Some(plugin_name) = cli.plugin.as_deref() {
            return self
                .resolve_non_interactive_request(cli, plugin_name, probe)
                .map(Some);
        }

        self.resolve_interactive_request(cli, probe)
    }

    /// Resolve a command-line plugin without consulting any interactive compatibility behavior.
    fn resolve_non_interactive_request(
        &self,
        cli: &Cli,
        plugin_name: &str,
        probe: &WorkflowToolchainProbe,
    ) -> Result<WorkflowRequest> {
        let build_mode = cli.build_mode();
        let archive_tool = cli.archive_tool();
        let request = WorkflowRequest::new(
            build_mode,
            archive_tool,
            PluginIdentity::parse(plugin_name),
            true,
            cli.resume_from,
            cli.fo4_dir.clone(),
        );
        Self::validate_non_interactive_candidate(&request, probe, self.ports.files.as_ref())?;

        Ok(request)
    }

    /// Resolve prompt-driven candidates through the existing interactive readiness adapter.
    ///
    /// Returns `None` when the user deliberately exits and otherwise returns the request after
    /// plugin readiness and resume intent have been resolved.
    fn resolve_interactive_request(
        &self,
        cli: &Cli,
        probe: &WorkflowToolchainProbe,
    ) -> Result<Option<WorkflowRequest>> {
        let build_mode = cli.build_mode();
        let archive_tool = cli.archive_tool();
        let mut resume_from = cli.resume_from;

        loop {
            let Some(plugin) = self.ports.prompts.prompt_plugin_name(build_mode)? else {
                return Ok(None);
            };

            let mut request = WorkflowRequest::new(
                build_mode,
                archive_tool,
                plugin,
                false,
                resume_from,
                cli.fo4_dir.clone(),
            );
            let readiness = Self::plugin_readiness(&request, probe)?;

            match self.ports.prompts.ensure_plugin_ready(&readiness)? {
                ExistingPluginAction::Exit => return Ok(None),
                ExistingPluginAction::ChooseResumeStep => {
                    if let Some(step) = self.ports.prompts.prompt_resume_step(build_mode)? {
                        request.resume_from = Some(step);
                    } else {
                        resume_from = None;
                        continue;
                    }
                }
                ExistingPluginAction::Continue => {}
            }

            return Ok(Some(request));
        }
    }

    /// Validate an unattended candidate and require its existing plugin before run preparation.
    fn validate_non_interactive_candidate(
        request: &WorkflowRequest,
        probe: &WorkflowToolchainProbe,
        files: &dyn FileSpace,
    ) -> Result<()> {
        let readiness = Self::plugin_readiness(request, probe)?;

        // Preserve the batch's failure precedence: an archive is actionable even when the
        // target plugin is also absent, so the target is deliberately not observed first.
        if files.is_file(&readiness.plugin_archive_path()) {
            return Err(Error::PluginAlreadyHasArchive);
        }

        let plugin_path = readiness.plugin_path();
        if !files.is_file(&plugin_path) {
            return Err(Error::Other(format!(
                "plugin not found: {}",
                plugin_path.display()
            )));
        }

        Ok(())
    }

    /// Validate a candidate and derive its probe-backed plugin-readiness view.
    fn plugin_readiness(
        request: &WorkflowRequest,
        probe: &WorkflowToolchainProbe,
    ) -> Result<PluginReadiness> {
        crate::validation::validate_plugin(&request.plugin, request.build_mode)?;
        Ok(probe.plugin_readiness(&request.plugin, request.non_interactive))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use super::*;
    use crate::config::{ArchiveTool, WorkflowStep};
    use crate::files::InMemoryFileSpace;

    #[derive(Debug, Default)]
    struct RecordingPrompts {
        plugin_names: RefCell<Vec<Option<PluginIdentity>>>,
        ready_actions: RefCell<Vec<ExistingPluginAction>>,
        resume_steps: RefCell<Vec<Option<WorkflowStep>>>,
        call_count: Cell<usize>,
    }

    impl RecordingPrompts {
        fn with_plugin_names(self, names: Vec<Option<PluginIdentity>>) -> Self {
            self.plugin_names.replace(names);
            self
        }

        fn with_ready_actions(self, actions: Vec<ExistingPluginAction>) -> Self {
            self.ready_actions.replace(actions);
            self
        }

        fn with_resume_steps(self, steps: Vec<Option<WorkflowStep>>) -> Self {
            self.resume_steps.replace(steps);
            self
        }

        fn call_count(&self) -> usize {
            self.call_count.get()
        }
    }

    impl WorkflowIntakePrompts for RecordingPrompts {
        fn prompt_plugin_name(&self, _build_mode: BuildMode) -> Result<Option<PluginIdentity>> {
            self.call_count.set(self.call_count.get() + 1);
            Ok(self.plugin_names.borrow_mut().remove(0))
        }

        fn ensure_plugin_ready(
            &self,
            _readiness: &PluginReadiness,
        ) -> Result<ExistingPluginAction> {
            self.call_count.set(self.call_count.get() + 1);
            Ok(self.ready_actions.borrow_mut().remove(0))
        }

        fn prompt_resume_step(&self, _build_mode: BuildMode) -> Result<Option<WorkflowStep>> {
            self.call_count.set(self.call_count.get() + 1);
            Ok(self.resume_steps.borrow_mut().remove(0))
        }
    }

    /// In-memory File Space that records readiness observations while retaining real file
    /// behavior for Workflow Run preparation and session-log assertions.
    #[derive(Debug, Default)]
    struct RecordingFileSpace {
        inner: InMemoryFileSpace,
        observed_paths: RefCell<Vec<PathBuf>>,
    }

    impl RecordingFileSpace {
        fn add_file(&self, path: impl Into<PathBuf>) {
            self.inner.add_file(path);
        }

        fn observed_paths(&self) -> Vec<PathBuf> {
            self.observed_paths.borrow().clone()
        }

        fn contains_file(&self, path: &Path) -> bool {
            self.inner.is_file(path)
        }

        fn contents(&self, path: &Path) -> String {
            self.inner.read_lossy(path).unwrap()
        }
    }

    impl FileSpace for RecordingFileSpace {
        fn is_file(&self, path: &Path) -> bool {
            self.observed_paths.borrow_mut().push(path.to_path_buf());
            self.inner.is_file(path)
        }

        fn find_first_file_with_extension(
            &self,
            directory: &Path,
            extension: &str,
        ) -> Option<PathBuf> {
            self.inner
                .find_first_file_with_extension(directory, extension)
        }

        fn remove_file(&self, path: &Path) -> Result<()> {
            self.inner.remove_file(path)
        }

        fn remove_dir_all(&self, directory: &Path) -> Result<()> {
            self.inner.remove_dir_all(directory)
        }

        fn read_lossy(&self, path: &Path) -> Result<String> {
            self.inner.read_lossy(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> Result<()> {
            self.inner.rename(from, to)
        }

        fn exists(&self, path: &Path) -> bool {
            self.inner.exists(path)
        }

        fn write(&self, path: &Path, contents: &str) -> Result<()> {
            self.inner.write(path, contents)
        }

        fn append(&self, path: &Path, contents: &str) -> Result<()> {
            self.inner.append(path, contents)
        }

        fn temp_dir(&self) -> PathBuf {
            self.inner.temp_dir()
        }
    }

    #[test]
    fn cli_plugin_continue_yields_non_interactive_request() {
        let cli = Cli::try_parse_from([
            "generateprevisibines",
            "-FiLtErEd",
            "-BsArCh",
            r"-fO4:C:\Fallout4",
            "MyMod",
        ])
        .unwrap();
        let prompts = Rc::new(RecordingPrompts::default());
        let files = Rc::new(InMemoryFileSpace::new());
        files.add_file(PathBuf::from(r"C:\Fallout4\Data\MyMod.esp"));
        let intake = WorkflowRequestIntake::new(prompts, files);
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(PathBuf::from(r"C:\Fallout4")),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();

        let request = intake.resolve_request(&cli, &probe).unwrap().unwrap();

        assert_eq!(request.build_mode, BuildMode::Filtered);
        assert_eq!(request.archive_tool, ArchiveTool::BSArch);
        assert_eq!(request.plugin.file_name, "MyMod.esp");
        assert_eq!(
            request.fallout4_override.as_deref(),
            Some(std::path::Path::new(r"C:\Fallout4"))
        );
        assert!(request.non_interactive);
    }

    /// Candidate validation wins before Intake asks the File Space any readiness question.
    #[test]
    fn invalid_non_interactive_candidate_fails_before_readiness_observations() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        let cli = Cli::try_parse_from(["generateprevisibines", "previs"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(RecordingPrompts::default());
        let files = Rc::new(RecordingFileSpace::default());
        files.add_file(data_dir.join("previs.esp"));
        files.add_file(data_dir.join("previs - Main.ba2"));
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(matches!(err, Error::ReservedPluginName { name } if name == "previs"));
        assert!(files.observed_paths().is_empty());
        assert_eq!(prompts.call_count(), 0);
    }

    /// The archive guard is intentionally evaluated before target-plugin existence.
    #[test]
    fn existing_plugin_archive_is_rejected_before_target_plugin_observation() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        let cli = Cli::try_parse_from(["generateprevisibines", "MyMod"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(RecordingPrompts::default());
        let files = Rc::new(RecordingFileSpace::default());
        let archive_path = data_dir.join("MyMod - Main.ba2");
        files.add_file(&archive_path);
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
        assert_eq!(files.observed_paths(), vec![archive_path]);
        assert_eq!(prompts.call_count(), 0);
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// A missing unattended plugin fails at Intake instead of consulting the compatibility
    /// prompt adapter, which would be able to block an automated invocation.
    #[test]
    fn missing_non_interactive_plugin_fails_without_prompting_or_preparing_a_run() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let cli = Cli::try_parse_from(["generateprevisibines", "MyMod"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir.clone()),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(RecordingPrompts::default());
        let files = Rc::new(RecordingFileSpace::default());
        files.add_file(fallout4_dir.join("Data").join("xPrevisPatch.esp"));
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(matches!(err, Error::Other(message) if message
                == format!("plugin not found: {}", fallout4_dir.join("Data").join("MyMod.esp").display())));
        assert_eq!(
            files.observed_paths(),
            vec![
                fallout4_dir.join("Data").join("MyMod - Main.ba2"),
                fallout4_dir.join("Data").join("MyMod.esp"),
            ]
        );
        assert_eq!(prompts.call_count(), 0);
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// A ready unattended request keeps its resume choice and creates its log through the same
    /// File Space handle that Intake used to establish plugin readiness.
    #[test]
    fn existing_non_interactive_plugin_uses_supplied_file_space_and_resume_intent() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        std::fs::create_dir_all(&fallout4_dir).unwrap();
        std::fs::write(fallout4_dir.join("CreationKit.exe"), b"").unwrap();
        std::fs::write(
            fallout4_dir.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();

        let cli =
            Cli::try_parse_from(["generateprevisibines", "--resume-from", "1", "MyMod"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir.clone()),
            creation_kit: Some(fallout4_dir.join("CreationKit.exe")),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(RecordingPrompts::default());
        let files = Rc::new(RecordingFileSpace::default());
        files.add_file(fallout4_dir.join("Data").join("MyMod.esp"));
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake.resolve(&cli, directory.path(), &probe).unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("an existing unattended plugin should produce a ready Workflow Run");
        };

        assert_eq!(
            run.config().resume_from,
            Some(WorkflowStep::GeneratePrecombines)
        );
        assert_eq!(prompts.call_count(), 0);
        assert_eq!(
            files.observed_paths(),
            vec![
                fallout4_dir.join("Data").join("MyMod - Main.ba2"),
                fallout4_dir.join("Data").join("MyMod.esp"),
            ]
        );
        assert!(files.contains_file(run.log_path()));
        assert_eq!(
            files.contents(run.log_path()),
            "Starting clean Build V2.95 of MyMod.esp\n"
        );
    }

    /// Leaving the interactive plugin prompt produces the named exit outcome without a log.
    #[test]
    fn interactive_plugin_exit_remains_named_and_does_not_prepare_a_run() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(RecordingPrompts::default().with_plugin_names(vec![None]));
        let files = Rc::new(RecordingFileSpace::default());
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake.resolve(&cli, directory.path(), &probe).unwrap();

        assert!(matches!(outcome, WorkflowIntakeOutcome::Exited));
        assert_eq!(prompts.call_count(), 1);
        assert!(files.observed_paths().is_empty());
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    #[test]
    fn interactive_resume_reprompt_clears_initial_resume_choice() {
        let cli = Cli::try_parse_from(["generateprevisibines", "--resume-from", "6", "--filtered"])
            .unwrap();
        let intake = WorkflowRequestIntake::new(
            Rc::new(
                RecordingPrompts::default()
                    .with_plugin_names(vec![
                        Some(PluginIdentity::parse("FirstMod")),
                        Some(PluginIdentity::parse("SecondMod")),
                    ])
                    .with_ready_actions(vec![
                        ExistingPluginAction::ChooseResumeStep,
                        ExistingPluginAction::Continue,
                    ])
                    .with_resume_steps(vec![None]),
            ),
            Rc::new(InMemoryFileSpace::new()),
        );
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(PathBuf::from(r"C:\Fallout4")),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();

        let request = intake.resolve_request(&cli, &probe).unwrap().unwrap();

        assert_eq!(request.plugin.file_name, "SecondMod.esp");
        assert_eq!(request.resume_from, None);
        assert!(!request.non_interactive);
    }

    #[test]
    fn interactive_resume_choice_updates_request() {
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let intake = WorkflowRequestIntake::new(
            Rc::new(
                RecordingPrompts::default()
                    .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                    .with_ready_actions(vec![ExistingPluginAction::ChooseResumeStep])
                    .with_resume_steps(vec![Some(WorkflowStep::GeneratePrevis)]),
            ),
            Rc::new(InMemoryFileSpace::new()),
        );
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(PathBuf::from(r"C:\Fallout4")),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();

        let request = intake.resolve_request(&cli, &probe).unwrap().unwrap();

        assert_eq!(request.plugin.file_name, "MyMod.esp");
        assert_eq!(request.resume_from, Some(WorkflowStep::GeneratePrevis));
        assert!(!request.non_interactive);
    }
}
