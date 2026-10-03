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
use crate::tools::wait::{MO2_DELAY_AFTER_SEED_COPY_SECS, SystemWait, Wait};

/// Result of Workflow Request Intake.
#[derive(Debug, Clone)]
pub enum WorkflowIntakeOutcome {
    /// A Workflow Run is prepared and ready to execute.
    Ready(Box<WorkflowRun>),
    /// The user deliberately exited during intake prompts.
    Exited,
}

/// Crate-private typed prompt seam used by Workflow Request Intake.
pub(crate) trait WorkflowIntakePrompts {
    /// Prompt for the plugin identity, or return `None` when the user exits.
    fn prompt_plugin_name(&self, build_mode: BuildMode) -> Result<Option<PluginIdentity>>;

    /// Report that the candidate plugin is missing before Intake evaluates the seed.
    fn report_missing_plugin(&self, plugin_file: &str);

    /// Ask whether a missing target should be copied from the seed plugin.
    ///
    /// Returns `true` when the user accepts the copy and `false` for a deliberate refusal.
    /// Prompt adapter failures propagate through Workflow Request Intake resolution.
    fn confirm_seed_copy(&self, plugin_file: &str) -> Result<bool>;

    /// Report that the accepted seed copy became visible and is ready to use.
    fn report_seed_copy_success(&self);

    /// Ask what Intake should do with a plugin it has already established exists.
    ///
    /// Returns the typed action selected by the user. Prompt adapter failures propagate through
    /// Workflow Request Intake resolution.
    fn prompt_existing_plugin_action(&self, plugin_file: &str) -> Result<ExistingPluginAction>;

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

    fn report_missing_plugin(&self, plugin_file: &str) {
        interactive::report_missing_plugin(plugin_file);
    }

    fn confirm_seed_copy(&self, _plugin_file: &str) -> Result<bool> {
        // The terminal already named the missing plugin via `report_missing_plugin`, so the
        // confirmation wording does not repeat it; the seam keeps the name for other adapters.
        interactive::prompt_seed_copy_confirmation()
    }

    fn report_seed_copy_success(&self) {
        interactive::report_seed_copy_success();
    }

    fn prompt_existing_plugin_action(&self, plugin_file: &str) -> Result<ExistingPluginAction> {
        interactive::prompt_existing_plugin_action(plugin_file)
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
    wait: Rc<dyn Wait>,
}

impl std::fmt::Debug for IntakePorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntakePorts")
            .field("prompts", &"shared WorkflowIntakePrompts")
            .field("files", &self.files)
            .field("wait", &self.wait)
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

    /// Create Intake from shared prompt and File Space adapters with the production wait.
    #[must_use]
    pub(crate) fn new<P, F>(prompts: Rc<P>, files: Rc<F>) -> Self
    where
        P: WorkflowIntakePrompts + 'static,
        F: FileSpace + 'static,
    {
        Self::new_with_wait(prompts, files, Rc::new(SystemWait))
    }

    /// Create Intake from owned shared adapters while allowing tests to retain typed handles.
    #[must_use]
    pub(crate) fn new_with_wait<P, F, W>(prompts: Rc<P>, files: Rc<F>, wait: Rc<W>) -> Self
    where
        P: WorkflowIntakePrompts + 'static,
        F: FileSpace + 'static,
        W: Wait + 'static,
    {
        Self {
            ports: IntakePorts {
                prompts,
                files,
                wait,
            },
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
        );
        Self::validate_non_interactive_candidate(&request, probe, self.ports.files.as_ref())?;

        Ok(request)
    }

    /// Resolve prompt-driven candidates through Intake-owned readiness and compatibility prompts.
    ///
    /// Returns `None` when the user deliberately exits and otherwise returns the request after
    /// plugin readiness and resume intent have been resolved. Validation, readiness, prompt,
    /// copying, and wait failures propagate without preparing a Workflow Run.
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

            let mut request =
                WorkflowRequest::new(build_mode, archive_tool, plugin, false, resume_from);
            crate::validation::validate_plugin(&request.plugin, request.build_mode)?;

            let data_dir = probe.data_dir();
            let archive_path = data_dir.join(request.plugin.archive_name());
            let plugin_path = data_dir.join(&request.plugin.file_name);
            let seed_path = data_dir.join("xPrevisPatch.esp");

            // Archive observation must precede target and seed observations so the actionable
            // conflict keeps the batch's established failure precedence. Whether it is a
            // conflict depends on where the run enters, which for an existing plugin is not
            // known until the operator answers below.
            let has_archive = self.ports.files.is_file(&archive_path);

            if !self.ports.files.is_file(&plugin_path) {
                // Seeding a plugin that already has an archive is always a conflict (batch 190).
                if has_archive {
                    return Err(Error::PluginAlreadyHasArchive);
                }

                self.ports
                    .prompts
                    .report_missing_plugin(&request.plugin.file_name);

                // Check that copying is possible before offering it; asking first would leave a
                // user approving an action Intake already knows cannot succeed.
                if !self.ports.files.is_file(&seed_path) {
                    return Err(Error::SeedPluginMissing);
                }

                if !self
                    .ports
                    .prompts
                    .confirm_seed_copy(&request.plugin.file_name)?
                {
                    return Ok(None);
                }

                self.ports.files.copy(&seed_path, &plugin_path)?;
                if !self.ports.files.is_file(&plugin_path) {
                    // MO2 may acknowledge the copy before its VFS exposes the target. Wait only
                    // for that state, then make one final observation to preserve the required
                    // copy-observe-wait-observe episode without delaying immediate copies.
                    self.ports.wait.sync_delay(MO2_DELAY_AFTER_SEED_COPY_SECS);
                    if !self.ports.files.is_file(&plugin_path) {
                        return Err(Error::SeedCopyFailed);
                    }
                }

                self.ports.prompts.report_seed_copy_success();
                return Ok(Some(request));
            }

            // Intake establishes readiness before asking a domain-shaped question, so prompt
            // adapters cannot inspect files or replace readiness policy with a canned result.
            match self
                .ports
                .prompts
                .prompt_existing_plugin_action(&request.plugin.file_name)?
            {
                ExistingPluginAction::Exit => return Ok(None),
                ExistingPluginAction::ChooseResumeStep => {
                    if let Some(step) = self.ports.prompts.prompt_resume_step(build_mode)? {
                        request.resume_from = Some(step);
                    } else {
                        // A newly selected plugin must not inherit resume intent chosen for the
                        // abandoned candidate; restarting Intake resets that decision to default.
                        resume_from = None;
                        continue;
                    }
                }
                ExistingPluginAction::Continue => {}
            }

            // Only a run entering Step 1 conflicts with the archive (batch 257). A resume at a
            // later step is how an operator recovers after Step 3 built it, so it must pass.
            if has_archive && enters_generate_precombines(request.resume_from) {
                return Err(Error::PluginAlreadyHasArchive);
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
        // Only a run entering Step 1 conflicts with it (batch 257); a `--resume-from` past
        // Step 1 is recovering from a failure after Step 3 built the archive.
        if enters_generate_precombines(request.resume_from)
            && files.is_file(&readiness.plugin_archive_path())
        {
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

/// Whether a run resumed at `resume_from` enters at Step 1, Generate Precombines.
///
/// A fresh run (`None`) and an explicit resume at Step 1 both do. This decides whether an
/// existing Plugin Archive conflicts with the run: the batch rejects one when entering Step 1
/// (`:Precomb1`, 257) and never on a `:GetStep` jump to a later step.
fn enters_generate_precombines(resume_from: Option<WorkflowStep>) -> bool {
    resume_from.is_none_or(|step| step == WorkflowStep::GeneratePrecombines)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use super::*;
    use crate::config::{ArchiveTool, WorkflowStep};
    use crate::files::InMemoryFileSpace;
    use crate::tools::wait::RecordingWait;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedQuestion {
        PluginName(BuildMode),
        SeedCopy(String),
        ExistingPlugin(String),
        ResumeStep(BuildMode),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ReadinessEvent {
        Observed(PathBuf),
        Copied(PathBuf, PathBuf),
        Waited(u64),
        Revealed(PathBuf),
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    enum CopyBehavior {
        #[default]
        Immediate,
        Pending,
        Invisible,
        Fail,
    }

    #[derive(Debug, Default)]
    struct RecordingPrompts {
        plugin_names: RefCell<Vec<Option<PluginIdentity>>>,
        seed_copy_confirmations: RefCell<Vec<bool>>,
        fail_seed_copy_confirmation: Cell<bool>,
        ready_actions: RefCell<Vec<ExistingPluginAction>>,
        resume_steps: RefCell<Vec<Option<WorkflowStep>>>,
        call_count: Cell<usize>,
        questions: RefCell<Vec<RecordedQuestion>>,
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

        fn with_seed_copy_confirmations(self, confirmations: Vec<bool>) -> Self {
            self.seed_copy_confirmations.replace(confirmations);
            self
        }

        fn with_failing_seed_copy_confirmation(self) -> Self {
            self.fail_seed_copy_confirmation.set(true);
            self
        }

        fn with_resume_steps(self, steps: Vec<Option<WorkflowStep>>) -> Self {
            self.resume_steps.replace(steps);
            self
        }

        fn call_count(&self) -> usize {
            self.call_count.get()
        }

        fn questions(&self) -> Vec<RecordedQuestion> {
            self.questions.borrow().clone()
        }
    }

    impl WorkflowIntakePrompts for RecordingPrompts {
        fn prompt_plugin_name(&self, build_mode: BuildMode) -> Result<Option<PluginIdentity>> {
            self.call_count.set(self.call_count.get() + 1);
            self.questions
                .borrow_mut()
                .push(RecordedQuestion::PluginName(build_mode));
            Ok(self.plugin_names.borrow_mut().remove(0))
        }

        fn confirm_seed_copy(&self, plugin_file: &str) -> Result<bool> {
            self.call_count.set(self.call_count.get() + 1);
            self.questions
                .borrow_mut()
                .push(RecordedQuestion::SeedCopy(plugin_file.to_owned()));
            if self.fail_seed_copy_confirmation.get() {
                return Err(Error::Other("recorded seed-copy prompt failure".to_owned()));
            }
            Ok(self.seed_copy_confirmations.borrow_mut().remove(0))
        }

        fn report_missing_plugin(&self, _plugin_file: &str) {}

        fn report_seed_copy_success(&self) {}

        fn prompt_existing_plugin_action(&self, plugin_file: &str) -> Result<ExistingPluginAction> {
            self.call_count.set(self.call_count.get() + 1);
            self.questions
                .borrow_mut()
                .push(RecordedQuestion::ExistingPlugin(plugin_file.to_owned()));
            Ok(self.ready_actions.borrow_mut().remove(0))
        }

        fn prompt_resume_step(&self, build_mode: BuildMode) -> Result<Option<WorkflowStep>> {
            self.call_count.set(self.call_count.get() + 1);
            self.questions
                .borrow_mut()
                .push(RecordedQuestion::ResumeStep(build_mode));
            Ok(self.resume_steps.borrow_mut().remove(0))
        }
    }

    /// In-memory File Space that records readiness observations while retaining real file
    /// behavior for Workflow Run preparation and session-log assertions.
    #[derive(Debug, Default)]
    struct RecordingFileSpace {
        inner: InMemoryFileSpace,
        observed_paths: RefCell<Vec<PathBuf>>,
        copied_paths: RefCell<Vec<(PathBuf, PathBuf)>>,
        copy_behavior: Cell<CopyBehavior>,
        fail_writes: Cell<bool>,
        pending_copy: RefCell<Option<(PathBuf, Option<Vec<u8>>)>>,
        events: Rc<RefCell<Vec<ReadinessEvent>>>,
    }

    impl RecordingFileSpace {
        /// Create a recording File Space whose copy visibility follows `copy_behavior`.
        fn with_copy_behavior(
            copy_behavior: CopyBehavior,
            events: Rc<RefCell<Vec<ReadinessEvent>>>,
        ) -> Self {
            Self {
                copy_behavior: Cell::new(copy_behavior),
                events,
                ..Self::default()
            }
        }

        fn add_file(&self, path: impl Into<PathBuf>) {
            self.inner.add_file(path);
        }

        /// Make session-log writes fail while leaving other File Space operations available.
        fn fail_writes(&self) {
            self.fail_writes.set(true);
        }

        fn add_file_with_bytes(&self, path: impl Into<PathBuf>, contents: impl Into<Vec<u8>>) {
            self.inner.add_file_with_bytes(path, contents);
        }

        fn observed_paths(&self) -> Vec<PathBuf> {
            self.observed_paths.borrow().clone()
        }

        fn copied_paths(&self) -> Vec<(PathBuf, PathBuf)> {
            self.copied_paths.borrow().clone()
        }

        fn contains_file(&self, path: &Path) -> bool {
            self.inner.is_file(path)
        }

        fn contents(&self, path: &Path) -> String {
            self.inner.read_lossy(path).unwrap()
        }

        fn bytes(&self, path: &Path) -> Option<Vec<u8>> {
            self.inner.contents(path)
        }

        /// Publish the pending opaque copy and record the simulated VFS reveal.
        fn reveal_pending_copy(&self) {
            let Some((path, contents)) = self.pending_copy.borrow_mut().take() else {
                return;
            };

            if let Some(contents) = contents {
                self.inner.add_file_with_bytes(&path, contents);
            } else {
                self.inner.add_file(&path);
            }
            self.events
                .borrow_mut()
                .push(ReadinessEvent::Revealed(path));
        }
    }

    impl FileSpace for RecordingFileSpace {
        fn is_file(&self, path: &Path) -> bool {
            self.observed_paths.borrow_mut().push(path.to_path_buf());
            self.events
                .borrow_mut()
                .push(ReadinessEvent::Observed(path.to_path_buf()));
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

        fn copy(&self, from: &Path, to: &Path) -> Result<()> {
            self.copied_paths
                .borrow_mut()
                .push((from.to_path_buf(), to.to_path_buf()));
            self.events
                .borrow_mut()
                .push(ReadinessEvent::Copied(from.to_path_buf(), to.to_path_buf()));

            match self.copy_behavior.get() {
                CopyBehavior::Immediate => self.inner.copy(from, to),
                CopyBehavior::Pending | CopyBehavior::Invisible => {
                    if !self.inner.is_file(from) {
                        return self.inner.copy(from, to);
                    }
                    self.pending_copy
                        .replace(Some((to.to_path_buf(), self.inner.contents(from))));
                    Ok(())
                }
                CopyBehavior::Fail => {
                    Err(std::io::Error::other("recorded seed copy failure").into())
                }
            }
        }

        fn rename(&self, from: &Path, to: &Path) -> Result<()> {
            self.inner.rename(from, to)
        }

        fn exists(&self, path: &Path) -> bool {
            self.inner.exists(path)
        }

        fn write(&self, path: &Path, contents: &str) -> Result<()> {
            if self.fail_writes.get() {
                return Err(std::io::Error::other("recorded session log write failure").into());
            }
            self.inner.write(path, contents)
        }

        fn append(&self, path: &Path, contents: &str) -> Result<()> {
            self.inner.append(path, contents)
        }

        fn temp_dir(&self) -> PathBuf {
            self.inner.temp_dir()
        }
    }

    /// Create the minimal real Creation Kit installation needed for ready-run preparation tests.
    fn create_ck_ready_fallout4(root: &Path) -> PathBuf {
        let fallout4_dir = root.join("Fallout4");
        std::fs::create_dir_all(&fallout4_dir).unwrap();
        std::fs::write(fallout4_dir.join("CreationKit.exe"), b"").unwrap();
        std::fs::write(
            fallout4_dir.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();
        fallout4_dir
    }

    /// Keep each case's CLI, probe, and installation together; a ready toolchain makes readiness
    /// failures prove their precedence over Workflow Run preparation.
    struct ReadyIntakeContext {
        directory: tempfile::TempDir,
        data_dir: PathBuf,
        cli: Cli,
        probe: WorkflowToolchainProbe,
    }

    impl ReadyIntakeContext {
        /// Prepare a distinct Creation Kit installation and parse this case's command-line input.
        fn new(args: &[&str]) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let fallout4_dir = create_ck_ready_fallout4(directory.path());
            let data_dir = fallout4_dir.join("Data");
            let cli = Cli::try_parse_from(args.iter().copied()).unwrap();
            let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
                creation_kit: Some(fallout4_dir.join("CreationKit.exe")),
                fo4edit: Some(crate::toolchain::write_fo4edit_install(
                    &directory.path().join("FO4Edit"),
                )),
                fallout4_dir: Some(fallout4_dir),
                ..crate::discovery::ToolPaths::default()
            })
            .unwrap();

            Self {
                directory,
                data_dir,
                cli,
                probe,
            }
        }
    }

    /// Command-line build mode, archive tool, and plugin choices reach the prepared unattended
    /// Workflow Run through the same resolution seam production uses.
    #[test]
    fn cli_plugin_choices_reach_the_prepared_non_interactive_run() {
        let context =
            ReadyIntakeContext::new(&["generateprevisibines", "-FiLtErEd", "-BsArCh", "MyMod"]);
        let prompts = Rc::new(RecordingPrompts::default());
        let files = Rc::new(InMemoryFileSpace::new());
        files.add_file(context.data_dir.join("MyMod.esp"));
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("an existing command-line plugin should prepare a Workflow Run");
        };

        let config = run.config();
        assert_eq!(config.build_mode, BuildMode::Filtered);
        assert_eq!(config.archive_tool, ArchiveTool::BSArch);
        assert_eq!(config.plugin.file_name, "MyMod.esp");
        assert!(config.non_interactive);
        assert_eq!(prompts.call_count(), 0);
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
        assert_eq!(files.observed_paths(), Vec::<PathBuf>::new());
        assert_eq!(prompts.call_count(), 0);
    }

    /// Recording prompts cannot bypass Intake validation: a prompted invalid candidate fails
    /// before archive, target, or seed observations and before any mutation.
    #[test]
    fn invalid_interactive_candidate_fails_before_readiness_observations() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("previs"))]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        let wait = Rc::new(RecordingWait::new());
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(matches!(err, Error::ReservedPluginName { name } if name == "previs"));
        assert_eq!(
            prompts.questions(),
            vec![RecordedQuestion::PluginName(BuildMode::Clean)]
        );
        assert_eq!(files.observed_paths(), Vec::<PathBuf>::new());
        assert_eq!(files.copied_paths(), Vec::<(PathBuf, PathBuf)>::new());
        assert_eq!(wait.delays(), Vec::<u64>::new());
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

    /// Prompted candidates keep archive precedence inside Intake rather than delegating it to a
    /// policy-bearing prompt adapter.
    ///
    /// The target is observed, because an existing plugin may still resume past Step 1 with
    /// its archive in place; a missing one is about to be seeded, which an archive always
    /// conflicts with (batch 190), so the rejection still precedes any seed observation.
    #[test]
    fn interactive_archive_is_rejected_before_seed_observation() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        let archive_path = data_dir.join("MyMod - Main.ba2");
        files.add_file(&archive_path);
        let wait = Rc::new(RecordingWait::new());
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
        // No `xPrevisPatch.esp` lookup: the seed is never observed.
        assert_eq!(
            files.observed_paths(),
            vec![archive_path, data_dir.join("MyMod.esp")]
        );
        assert_eq!(files.copied_paths(), Vec::<(PathBuf, PathBuf)>::new());
        assert_eq!(wait.delays(), Vec::<u64>::new());
        assert_eq!(
            prompts.questions(),
            vec![RecordedQuestion::PluginName(BuildMode::Clean)]
        );
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
            fo4edit: Some(crate::toolchain::write_fo4edit_install(
                &directory.path().join("FO4Edit"),
            )),
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
            "Starting clean Build V2.99 of MyMod.esp\n"
        );
    }

    /// Continuing with an existing prompted plugin preserves the caller's current resume intent
    /// and prepares the Workflow Run through the Intake resolution seam.
    #[test]
    fn existing_interactive_plugin_continues_with_supplied_resume_intent() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = create_ck_ready_fallout4(directory.path());
        let data_dir = fallout4_dir.join("Data");

        let cli = Cli::try_parse_from(["generateprevisibines", "--resume-from", "1"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir.clone()),
            creation_kit: Some(fallout4_dir.join("CreationKit.exe")),
            fo4edit: Some(crate::toolchain::write_fo4edit_install(
                &directory.path().join("FO4Edit"),
            )),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_ready_actions(vec![ExistingPluginAction::Continue]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        files.add_file(data_dir.join("MyMod.esp"));
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake.resolve(&cli, directory.path(), &probe).unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("continuing with an existing plugin should prepare a Workflow Run");
        };

        assert_eq!(
            run.config().resume_from,
            Some(WorkflowStep::GeneratePrecombines)
        );
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::ExistingPlugin("MyMod.esp".into()),
            ]
        );
        assert!(files.contains_file(run.log_path()));
    }

    /// Exiting from the existing-plugin question remains a deliberate Intake outcome and never
    /// reaches Workflow Run preparation or session-log creation.
    #[test]
    fn exiting_existing_interactive_plugin_returns_named_exit_without_preparing_run() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_ready_actions(vec![ExistingPluginAction::Exit]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        files.add_file(data_dir.join("MyMod.esp"));
        files.fail_writes();
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake.resolve(&cli, directory.path(), &probe).unwrap();

        assert!(matches!(outcome, WorkflowIntakeOutcome::Exited));
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::ExistingPlugin("MyMod.esp".into()),
            ]
        );
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// An empty plugin-name answer produces the named exit outcome without attempting Workflow
    /// Run preparation or creating a session log.
    #[test]
    fn empty_plugin_name_answer_returns_named_exit_without_preparing_run() {
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
        files.fail_writes();
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake.resolve(&cli, directory.path(), &probe).unwrap();

        assert!(matches!(outcome, WorkflowIntakeOutcome::Exited));
        assert_eq!(
            prompts.questions(),
            vec![RecordedQuestion::PluginName(BuildMode::Clean)]
        );
        assert_eq!(files.observed_paths(), Vec::<PathBuf>::new());
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// A missing seed is an Intake failure discovered before asking whether an impossible copy
    /// should be attempted, and before Workflow Run preparation creates a session log.
    #[test]
    fn missing_interactive_seed_fails_before_copy_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(matches!(err, Error::SeedPluginMissing));
        assert_eq!(
            files.observed_paths(),
            vec![
                data_dir.join("MyMod - Main.ba2"),
                data_dir.join("MyMod.esp"),
                data_dir.join("xPrevisPatch.esp"),
            ]
        );
        assert_eq!(prompts.call_count(), 1);
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// A typed prompt failure propagates unchanged and prevents copying, waiting, and run
    /// preparation.
    #[test]
    fn seed_copy_prompt_failure_remains_visible_through_resolution() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_failing_seed_copy_confirmation(),
        );
        let files = Rc::new(RecordingFileSpace::default());
        files.add_file_with_bytes(data_dir.join("xPrevisPatch.esp"), [0x00, 0xFF, 0x80]);
        let wait = Rc::new(RecordingWait::new());
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(
            matches!(err, Error::Other(message) if message == "recorded seed-copy prompt failure")
        );
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::SeedCopy("MyMod.esp".into()),
            ]
        );
        assert_eq!(files.copied_paths(), Vec::<(PathBuf, PathBuf)>::new());
        assert_eq!(wait.delays(), Vec::<u64>::new());
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// Declining the typed seed-copy question is a deliberate exit, not an operational error.
    #[test]
    fn declining_interactive_seed_copy_exits_without_preparing_a_run() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_seed_copy_confirmations(vec![false]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        files.add_file_with_bytes(data_dir.join("xPrevisPatch.esp"), [0x00, 0xFF, 0x80]);
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake.resolve(&cli, directory.path(), &probe).unwrap();

        assert!(matches!(outcome, WorkflowIntakeOutcome::Exited));
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::SeedCopy("MyMod.esp".into()),
            ]
        );
        assert_eq!(
            files.observed_paths(),
            vec![
                data_dir.join("MyMod - Main.ba2"),
                data_dir.join("MyMod.esp"),
                data_dir.join("xPrevisPatch.esp"),
            ]
        );
        assert!(!files.contains_file(&data_dir.join("MyMod.esp")));
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// An immediately visible opaque copy proceeds directly to Workflow Run preparation without
    /// the MO2 delay or the existing-plugin compatibility question.
    #[test]
    fn immediately_visible_seed_copy_preserves_bytes_and_prepares_a_run_without_waiting() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        std::fs::create_dir_all(&fallout4_dir).unwrap();
        std::fs::write(fallout4_dir.join("CreationKit.exe"), b"").unwrap();
        std::fs::write(
            fallout4_dir.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();

        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir.clone()),
            creation_kit: Some(fallout4_dir.join("CreationKit.exe")),
            fo4edit: Some(crate::toolchain::write_fo4edit_install(
                &directory.path().join("FO4Edit"),
            )),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_seed_copy_confirmations(vec![true]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        let wait = Rc::new(RecordingWait::new());
        let seed_path = data_dir.join("xPrevisPatch.esp");
        let plugin_path = data_dir.join("MyMod.esp");
        let seed_bytes = vec![0x00, 0xFF, 0x80, b'E', b'S', b'P'];
        files.add_file_with_bytes(&seed_path, seed_bytes.clone());
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let outcome = intake.resolve(&cli, directory.path(), &probe).unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("a visible copied plugin should prepare a Workflow Run");
        };

        assert_eq!(run.config().plugin.file_name, "MyMod.esp");
        assert_eq!(files.bytes(&plugin_path), Some(seed_bytes));
        assert_eq!(
            files.copied_paths(),
            vec![(seed_path.clone(), plugin_path.clone())]
        );
        assert_eq!(
            files.observed_paths(),
            vec![
                data_dir.join("MyMod - Main.ba2"),
                plugin_path.clone(),
                seed_path,
                plugin_path,
            ]
        );
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::SeedCopy("MyMod.esp".into()),
            ]
        );
        assert_eq!(wait.delays(), Vec::<u64>::new());
        assert!(files.contains_file(run.log_path()));
    }

    /// Toolchain preparation errors after successful readiness propagate through Intake and do
    /// not leave a session log behind.
    #[test]
    fn copied_plugin_toolchain_failure_remains_visible_without_creating_a_log() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_seed_copy_confirmations(vec![true]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        let wait = Rc::new(RecordingWait::new());
        let seed_path = data_dir.join("xPrevisPatch.esp");
        let plugin_path = data_dir.join("MyMod.esp");
        files.add_file_with_bytes(&seed_path, [0x00, 0xFF, 0x80]);
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(matches!(err, Error::Other(message) if message
                == format!("CreationKit.exe not found in {}", data_dir.parent().unwrap().display())));
        assert_eq!(files.bytes(&plugin_path), Some(vec![0x00, 0xFF, 0x80]));
        assert_eq!(wait.delays(), Vec::<u64>::new());
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// Session-log initialization errors remain Workflow Run preparation failures after the
    /// copied plugin is ready; Intake does not turn them into a ready or exited outcome.
    #[test]
    fn copied_plugin_log_write_failure_remains_visible_through_resolution() {
        let directory = tempfile::tempdir().unwrap();
        let fallout4_dir = directory.path().join("Fallout4");
        let data_dir = fallout4_dir.join("Data");
        std::fs::create_dir_all(&fallout4_dir).unwrap();
        std::fs::write(fallout4_dir.join("CreationKit.exe"), b"").unwrap();
        std::fs::write(
            fallout4_dir.join("fallout4_test.ini"),
            "[CreationKit]\nBSHandleRefObjectPatch=true\n[CreationKit_Log]\nOutputFile=CK.log\n",
        )
        .unwrap();

        let cli = Cli::try_parse_from(["generateprevisibines"]).unwrap();
        let probe = WorkflowToolchainProbe::from_tool_paths(crate::discovery::ToolPaths {
            fallout4_dir: Some(fallout4_dir.clone()),
            creation_kit: Some(fallout4_dir.join("CreationKit.exe")),
            fo4edit: Some(crate::toolchain::write_fo4edit_install(
                &directory.path().join("FO4Edit"),
            )),
            ..crate::discovery::ToolPaths::default()
        })
        .unwrap();
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_seed_copy_confirmations(vec![true]),
        );
        let files = Rc::new(RecordingFileSpace::default());
        let wait = Rc::new(RecordingWait::new());
        let seed_path = data_dir.join("xPrevisPatch.esp");
        let plugin_path = data_dir.join("MyMod.esp");
        files.add_file_with_bytes(&seed_path, [0x00, 0xFF, 0x80]);
        files.fail_writes();
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let err = intake.resolve(&cli, directory.path(), &probe).unwrap_err();

        assert!(matches!(err, Error::Io(error) if error.to_string()
                == "recorded session log write failure"));
        assert_eq!(files.bytes(&plugin_path), Some(vec![0x00, 0xFF, 0x80]));
        assert_eq!(wait.delays(), Vec::<u64>::new());
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// A copied plugin remains pending until the Recording Wait effect exposes it, pinning the
    /// required MO2 copy-observe-wait-reveal-observe ordering without a real sleep.
    #[test]
    fn delayed_seed_copy_waits_once_then_observes_the_revealed_plugin() {
        let context = ReadyIntakeContext::new(&["generateprevisibines"]);
        let data_dir = &context.data_dir;
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_seed_copy_confirmations(vec![true]),
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let files = Rc::new(RecordingFileSpace::with_copy_behavior(
            CopyBehavior::Pending,
            Rc::clone(&events),
        ));
        let seed_path = data_dir.join("xPrevisPatch.esp");
        let plugin_path = data_dir.join("MyMod.esp");
        let seed_bytes = vec![0x00, 0xFF, 0x80, b'E', b'S', b'P'];
        files.add_file_with_bytes(&seed_path, seed_bytes.clone());
        let effect_events = Rc::clone(&events);
        let effect_files = Rc::clone(&files);
        let wait = Rc::new(RecordingWait::with_effect(move |seconds| {
            effect_events
                .borrow_mut()
                .push(ReadinessEvent::Waited(seconds));
            effect_files.reveal_pending_copy();
        }));
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let outcome = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("a plugin revealed by the MO2 wait should prepare a Workflow Run");
        };

        assert_eq!(wait.delays(), vec![MO2_DELAY_AFTER_SEED_COPY_SECS]);
        assert_eq!(files.bytes(&plugin_path), Some(seed_bytes));
        assert_eq!(
            *events.borrow(),
            vec![
                ReadinessEvent::Observed(data_dir.join("MyMod - Main.ba2")),
                ReadinessEvent::Observed(plugin_path.clone()),
                ReadinessEvent::Observed(seed_path.clone()),
                ReadinessEvent::Copied(seed_path, plugin_path.clone()),
                ReadinessEvent::Observed(plugin_path.clone()),
                ReadinessEvent::Waited(MO2_DELAY_AFTER_SEED_COPY_SECS),
                ReadinessEvent::Revealed(plugin_path.clone()),
                ReadinessEvent::Observed(plugin_path),
            ]
        );
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::SeedCopy("MyMod.esp".into()),
            ]
        );
        assert!(files.contains_file(run.log_path()));
    }

    /// A copied target that remains invisible after the one mandated wait fails readiness and
    /// never reaches Workflow Run preparation.
    #[test]
    fn invisible_seed_copy_fails_after_one_wait_and_one_post_wait_observation() {
        let context = ReadyIntakeContext::new(&["generateprevisibines"]);
        let data_dir = &context.data_dir;
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_seed_copy_confirmations(vec![true]),
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let files = Rc::new(RecordingFileSpace::with_copy_behavior(
            CopyBehavior::Invisible,
            Rc::clone(&events),
        ));
        let seed_path = data_dir.join("xPrevisPatch.esp");
        let plugin_path = data_dir.join("MyMod.esp");
        files.add_file_with_bytes(&seed_path, [0x00, 0xFF, 0x80]);
        let effect_events = Rc::clone(&events);
        let wait = Rc::new(RecordingWait::with_effect(move |seconds| {
            effect_events
                .borrow_mut()
                .push(ReadinessEvent::Waited(seconds));
        }));
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let err = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap_err();

        assert!(matches!(err, Error::SeedCopyFailed));
        assert_eq!(wait.delays(), vec![MO2_DELAY_AFTER_SEED_COPY_SECS]);
        assert_eq!(
            *events.borrow(),
            vec![
                ReadinessEvent::Observed(data_dir.join("MyMod - Main.ba2")),
                ReadinessEvent::Observed(plugin_path.clone()),
                ReadinessEvent::Observed(seed_path.clone()),
                ReadinessEvent::Copied(seed_path, plugin_path.clone()),
                ReadinessEvent::Observed(plugin_path.clone()),
                ReadinessEvent::Waited(MO2_DELAY_AFTER_SEED_COPY_SECS),
                ReadinessEvent::Observed(plugin_path),
            ]
        );
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// File Space copy failures remain the original I/O error and short-circuit visibility and
    /// wait behavior.
    #[test]
    fn seed_copy_failure_propagates_without_observing_or_waiting_again() {
        let context = ReadyIntakeContext::new(&["generateprevisibines"]);
        let data_dir = &context.data_dir;
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_seed_copy_confirmations(vec![true]),
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let files = Rc::new(RecordingFileSpace::with_copy_behavior(
            CopyBehavior::Fail,
            Rc::clone(&events),
        ));
        let seed_path = data_dir.join("xPrevisPatch.esp");
        let plugin_path = data_dir.join("MyMod.esp");
        files.add_file_with_bytes(&seed_path, [0x00, 0xFF, 0x80]);
        let wait = Rc::new(RecordingWait::new());
        let intake = WorkflowRequestIntake::new_with_wait(
            Rc::clone(&prompts),
            Rc::clone(&files),
            Rc::clone(&wait),
        );

        let err = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap_err();

        assert!(
            matches!(err, Error::Io(error) if error.to_string() == "recorded seed copy failure")
        );
        assert_eq!(wait.delays(), Vec::<u64>::new());
        assert_eq!(
            *events.borrow(),
            vec![
                ReadinessEvent::Observed(data_dir.join("MyMod - Main.ba2")),
                ReadinessEvent::Observed(plugin_path.clone()),
                ReadinessEvent::Observed(seed_path.clone()),
                ReadinessEvent::Copied(seed_path, plugin_path),
            ]
        );
        assert!(!files.contains_file(&files.temp_dir().join("MyMod.log")));
    }

    /// Re-entering the plugin name clears inherited resume intent before Intake resolves and
    /// prepares the next candidate.
    #[test]
    fn reentering_plugin_name_clears_inherited_resume_before_next_candidate() {
        let context =
            ReadyIntakeContext::new(&["generateprevisibines", "--resume-from", "6", "--filtered"]);
        let files = Rc::new(InMemoryFileSpace::new());
        files.add_file(context.data_dir.join("FirstMod.esp"));
        files.add_file(context.data_dir.join("SecondMod.esp"));
        let prompts = Rc::new(
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
        );
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("the replacement plugin should prepare a Workflow Run");
        };

        assert_eq!(run.config().plugin.file_name, "SecondMod.esp");
        assert_eq!(run.config().resume_from, None);
        assert_eq!(
            run.planned_steps().first(),
            Some(&WorkflowStep::GeneratePrecombines)
        );
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Filtered),
                RecordedQuestion::ExistingPlugin("FirstMod.esp".into()),
                RecordedQuestion::ResumeStep(BuildMode::Filtered),
                RecordedQuestion::PluginName(BuildMode::Filtered),
                RecordedQuestion::ExistingPlugin("SecondMod.esp".into()),
            ]
        );
        assert!(files.is_file(run.log_path()));
    }

    /// Choosing a resume step updates the Workflow Request before Intake prepares the Workflow
    /// Run, so both its configuration and Workflow Plan begin at the selected step.
    #[test]
    fn choosing_interactive_resume_step_updates_prepared_run() {
        let context = ReadyIntakeContext::new(&["generateprevisibines", "--resume-from", "6"]);
        let files = Rc::new(InMemoryFileSpace::new());
        files.add_file(context.data_dir.join("MyMod.esp"));
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_ready_actions(vec![ExistingPluginAction::ChooseResumeStep])
                .with_resume_steps(vec![Some(WorkflowStep::GeneratePrecombines)]),
        );
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("choosing a supported resume step should prepare a Workflow Run");
        };

        assert_eq!(run.config().plugin.file_name, "MyMod.esp");
        assert_eq!(
            run.config().resume_from,
            Some(WorkflowStep::GeneratePrecombines)
        );
        assert_eq!(
            run.planned_steps().first(),
            Some(&WorkflowStep::GeneratePrecombines)
        );
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::ExistingPlugin("MyMod.esp".into()),
                RecordedQuestion::ResumeStep(BuildMode::Clean),
            ]
        );
        assert!(files.is_file(run.log_path()));
    }

    /// A resume past Step 1 keeps the Plugin Archive Step 3 built: the batch only rejects an
    /// archive when seeding a missing plugin (190) or entering Step 1 (257), and a `:GetStep`
    /// jump to a later step reaches neither.
    #[test]
    fn a_non_interactive_resume_past_step_one_keeps_the_existing_plugin_archive() {
        let context =
            ReadyIntakeContext::new(&["generateprevisibines", "--resume-from", "4", "MyMod"]);
        let prompts = Rc::new(RecordingPrompts::default());
        let files = Rc::new(InMemoryFileSpace::new());
        files.add_file(context.data_dir.join("MyMod.esp"));
        files.add_file(context.data_dir.join("MyMod - Main.ba2"));
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("a resume at Step 4 should prepare a Workflow Run despite the archive");
        };

        assert_eq!(run.config().resume_from, Some(WorkflowStep::CompressPsg));
        assert_eq!(
            run.runnable_steps(),
            &[
                WorkflowStep::CompressPsg,
                WorkflowStep::BuildCdx,
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
            ]
        );
        assert_eq!(prompts.call_count(), 0);
    }

    /// An explicit resume at Step 1 still enters Step 1, so the archive is still rejected there.
    #[test]
    fn a_non_interactive_resume_at_step_one_still_rejects_an_existing_plugin_archive() {
        let context =
            ReadyIntakeContext::new(&["generateprevisibines", "--resume-from", "1", "MyMod"]);
        let prompts = Rc::new(RecordingPrompts::default());
        let files = Rc::new(InMemoryFileSpace::new());
        files.add_file(context.data_dir.join("MyMod.esp"));
        files.add_file(context.data_dir.join("MyMod - Main.ba2"));
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let err = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
        assert!(!files.is_file(&files.temp_dir().join("MyMod.log")));
    }

    /// The menu's "Rerun from failed step" is the advertised recovery after a Step 4 failure,
    /// and by then Step 3 has built the archive; Intake must not reject it before the operator
    /// has said which step to rerun.
    #[test]
    fn an_interactive_resume_past_step_one_keeps_the_existing_plugin_archive() {
        let context = ReadyIntakeContext::new(&["generateprevisibines"]);
        let files = Rc::new(InMemoryFileSpace::new());
        files.add_file(context.data_dir.join("MyMod.esp"));
        files.add_file(context.data_dir.join("MyMod - Main.ba2"));
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_ready_actions(vec![ExistingPluginAction::ChooseResumeStep])
                .with_resume_steps(vec![Some(WorkflowStep::CompressPsg)]),
        );
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let outcome = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap();
        let WorkflowIntakeOutcome::Ready(run) = outcome else {
            panic!("a resume at Step 4 should prepare a Workflow Run despite the archive");
        };

        assert_eq!(run.config().resume_from, Some(WorkflowStep::CompressPsg));
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::ExistingPlugin("MyMod.esp".into()),
                RecordedQuestion::ResumeStep(BuildMode::Clean),
            ]
        );
    }

    /// Using an existing plugin from the start enters Step 1, so the archive is rejected — once
    /// the operator has answered, and still before any Workflow Run is prepared.
    #[test]
    fn an_interactive_fresh_run_still_rejects_an_existing_plugin_archive() {
        let context = ReadyIntakeContext::new(&["generateprevisibines"]);
        let files = Rc::new(InMemoryFileSpace::new());
        files.add_file(context.data_dir.join("MyMod.esp"));
        files.add_file(context.data_dir.join("MyMod - Main.ba2"));
        let prompts = Rc::new(
            RecordingPrompts::default()
                .with_plugin_names(vec![Some(PluginIdentity::parse("MyMod"))])
                .with_ready_actions(vec![ExistingPluginAction::Continue]),
        );
        let intake = WorkflowRequestIntake::new(Rc::clone(&prompts), Rc::clone(&files));

        let err = intake
            .resolve(&context.cli, context.directory.path(), &context.probe)
            .unwrap_err();

        assert!(matches!(err, Error::PluginAlreadyHasArchive));
        assert_eq!(
            prompts.questions(),
            vec![
                RecordedQuestion::PluginName(BuildMode::Clean),
                RecordedQuestion::ExistingPlugin("MyMod.esp".into()),
            ]
        );
        assert!(!files.is_file(&files.temp_dir().join("MyMod.log")));
    }
}
