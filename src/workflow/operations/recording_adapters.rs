//! The shared recording [`OperationAdapters`] used by Workflow Operation tests.
//!
//! One fake, not one per module: a change to how Creation Kit's effects are simulated
//! happens here and nowhere else. It is crate-visible rather than nested in a private
//! `tests` module so the Workflow Run module's tests can drive the same simulation.
//!
//! The fake owns the [`InMemoryFileSpace`] it hands back from [`OperationAdapters::files`],
//! so the Creation Kit simulation and the Precombine Workspace observe the same space by
//! construction — no shared-ownership plumbing, and no fabricated files on disk. A test
//! states artifact outcomes ("Creation Kit ran and produced meshes but no geometry PSG")
//! instead of writing bytes into a temporary directory and hoping the workspace finds them.

use std::cell::{Cell, RefCell};
use std::path::Path;

use crate::config::BuildMode;
use crate::error::Result;
use crate::files::{FileSpace, InMemoryFileSpace};
use crate::run::WorkflowRun;
use crate::tools::CkOperation;

use super::OperationAdapters;

/// One Creation Kit invocation as the operation under test issued it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedCreationKitCall {
    pub(crate) operation: CkOperation,
    pub(crate) plugin_file: String,
    pub(crate) qualifiers: String,
}

/// Whether a simulated Creation Kit run leaves a given artifact behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactOutcome {
    Present,
    Absent,
}

impl ArtifactOutcome {
    const fn is_present(self) -> bool {
        matches!(self, Self::Present)
    }
}

/// The artifact outcomes a simulated Creation Kit run declares.
///
/// A successful Step 1 run is the default; each builder method on the adapter withholds
/// exactly one artifact so a test names the failure it is about.
#[derive(Debug, Clone, Copy)]
struct GeneratedArtifacts {
    combined_objects: ArtifactOutcome,
    precombined_mesh: ArtifactOutcome,
    geometry_psg: ArtifactOutcome,
}

impl Default for GeneratedArtifacts {
    fn default() -> Self {
        Self {
            combined_objects: ArtifactOutcome::Present,
            precombined_mesh: ArtifactOutcome::Present,
            geometry_psg: ArtifactOutcome::Present,
        }
    }
}

/// A quiet Creation Kit log: present, readable, and free of the handle-array marker.
const QUIET_CK_LOG: &str = "Masterfile: Fallout4.esm\n";

/// Test [`OperationAdapters`] that record what they were asked to do and declare outcomes.
#[derive(Debug)]
pub(crate) struct RecordingOperationAdapters {
    creation_kit_calls: RefCell<Vec<RecordedCreationKitCall>>,
    clear_prompts: Cell<usize>,
    clear_response: bool,
    artifacts: GeneratedArtifacts,
    ck_log_contents: String,
    files: InMemoryFileSpace,
}

impl Default for RecordingOperationAdapters {
    fn default() -> Self {
        Self {
            creation_kit_calls: RefCell::new(Vec::new()),
            clear_prompts: Cell::new(0),
            clear_response: true,
            artifacts: GeneratedArtifacts::default(),
            ck_log_contents: QUIET_CK_LOG.to_owned(),
            files: InMemoryFileSpace::new(),
        }
    }
}

impl RecordingOperationAdapters {
    /// Adapters simulating a fully successful Creation Kit run over an empty space.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Simulate a Creation Kit run that leaves no merged objects plugin behind.
    #[must_use]
    pub(crate) const fn without_combined_objects(mut self) -> Self {
        self.artifacts.combined_objects = ArtifactOutcome::Absent;
        self
    }

    /// Simulate a Creation Kit run that leaves no precombined meshes behind.
    #[must_use]
    pub(crate) const fn without_precombined_meshes(mut self) -> Self {
        self.artifacts.precombined_mesh = ArtifactOutcome::Absent;
        self
    }

    /// Simulate a Creation Kit run that leaves no geometry PSG behind.
    #[must_use]
    pub(crate) const fn without_geometry_psg(mut self) -> Self {
        self.artifacts.geometry_psg = ArtifactOutcome::Absent;
        self
    }

    /// Give the simulated Creation Kit log the supplied contents.
    #[must_use]
    pub(crate) fn with_ck_log_contents(mut self, contents: impl Into<String>) -> Self {
        self.ck_log_contents = contents.into();
        self
    }

    /// Answer the clear-precombined prompt with a refusal instead of consent.
    #[must_use]
    pub(crate) const fn refusing_clear_precombined(mut self) -> Self {
        self.clear_response = false;
        self
    }

    /// The [`FileSpace`] this fake and the operation under test share.
    ///
    /// Tests seed preconditions and assert outcomes through it directly. Deliberately not
    /// called "the artifact space": the glossary binds that phrase to the Precombine
    /// Workspace, which is a different thing evaluated *through* this space.
    #[must_use]
    pub(crate) const fn file_space(&self) -> &InMemoryFileSpace {
        &self.files
    }

    /// The Creation Kit invocations recorded so far, in call order.
    #[must_use]
    pub(crate) fn creation_kit_calls(&self) -> Vec<RecordedCreationKitCall> {
        self.creation_kit_calls.borrow().clone()
    }

    /// How many times the clear-precombined prompt was shown.
    #[must_use]
    pub(crate) fn clear_prompt_count(&self) -> usize {
        self.clear_prompts.get()
    }
}

impl OperationAdapters for RecordingOperationAdapters {
    fn files(&self) -> &dyn FileSpace {
        &self.files
    }

    fn run_creation_kit(
        &self,
        run: &WorkflowRun,
        operation: CkOperation,
        plugin_file: &str,
        qualifiers: &str,
    ) -> Result<()> {
        self.creation_kit_calls
            .borrow_mut()
            .push(RecordedCreationKitCall {
                operation,
                plugin_file: plugin_file.to_owned(),
                qualifiers: qualifiers.to_owned(),
            });

        // The fake supplies Creation Kit's external outputs while the real operation keeps
        // ownership of validation: it declares which artifacts appear, never whether the
        // set of them is enough for Step 1 to pass.
        let config = run.config();

        if self.artifacts.combined_objects.is_present() {
            self.files
                .add_file(config.fo4edit_data_dir().join("CombinedObjects.esp"));
        }

        if self.artifacts.precombined_mesh.is_present() {
            // Nested, because that is the shape Creation Kit writes precombines in.
            self.files
                .add_file(config.precombined_dir().join("cell").join("mesh.nif"));
        }

        // Only a Clean-mode run emits the geometry PSG; a Filtered one never does.
        if config.build_mode == BuildMode::Clean && self.artifacts.geometry_psg.is_present() {
            self.files.add_file(
                config
                    .fo4edit_data_dir()
                    .join(format!("{} - Geometry.psg", config.plugin.base_name)),
            );
        }

        self.files
            .add_file_with_contents(&run.tool_context().ck_log_path, self.ck_log_contents.clone());

        Ok(())
    }

    fn confirm_clear_precombined(&self, _precombined_dir: &Path) -> Result<bool> {
        self.clear_prompts.set(self.clear_prompts.get() + 1);
        Ok(self.clear_response)
    }
}
