//! Test doubles shared by the Workflow Operation and Workflow Run test modules.
//!
//! Crate-visible rather than nested in a private `tests` module so the Workflow Run module's
//! tests can drive the same Step 1 simulation the operation's own tests do.
//!
//! There is no recording Creation Kit here, and that is the point. Substitution happens
//! *underneath* the Creation Kit episode, at `RecordingProcessRunner` and `RecordingWait`, so
//! the real `CreationKitOps` runs and a Step 1 test observes the DLL guard, the mandated MO2
//! delay and the log lifecycle rather than a fake's claim about them. What this module supplies
//! is the other half: the [`Prompts`] the operation asks, and the files Creation Kit would have
//! left behind — recorded into the space the operation reads back, so a test states an outcome
//! instead of writing bytes into a temporary directory and hoping the workspace finds them.

use std::cell::Cell;
use std::path::Path;

use crate::config::{BuildMode, ProjectConfig};
use crate::error::Result;
use crate::files::InMemoryFileSpace;

use super::Prompts;

/// A quiet Creation Kit log: present, readable, and free of the handle-array marker.
pub(crate) const QUIET_CK_LOG: &str = "Masterfile: Fallout4.esm\n";

/// Test [`Prompts`] that count the confirmations they were asked and answer from a script.
#[derive(Debug)]
pub(crate) struct RecordingPrompts {
    clear_prompts: Cell<usize>,
    clear_response: bool,
}

impl Default for RecordingPrompts {
    fn default() -> Self {
        Self {
            clear_prompts: Cell::new(0),
            // Consent by default, so a refusal is something a test has to ask for by name.
            clear_response: true,
        }
    }
}

impl RecordingPrompts {
    /// Prompts that have been asked nothing yet and consent to what they are asked.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Answer the clear-precombined prompt with a refusal instead of consent.
    #[must_use]
    pub(crate) const fn refusing_clear_precombined(mut self) -> Self {
        self.clear_response = false;
        self
    }

    /// How many times the clear-precombined prompt was shown.
    #[must_use]
    pub(crate) fn clear_prompt_count(&self) -> usize {
        self.clear_prompts.get()
    }
}

impl Prompts for RecordingPrompts {
    fn confirm_clear_precombined(&self, _precombined_dir: &Path) -> Result<bool> {
        self.clear_prompts.set(self.clear_prompts.get() + 1);
        Ok(self.clear_response)
    }
}

/// Record what a successful Creation Kit precombine run leaves in a Workflow Run's space.
///
/// Meant as a `RecordingProcessRunner` effects callback body, so the simulated tool writes into
/// the very space the Precombine Workspace reads back — the same relationship the real pair
/// has. One definition, not one per test module: a change to what Creation Kit produces belongs
/// here and nowhere else.
///
/// `ck_log` is the log CKPE configured Creation Kit to write. The episode deletes it before the
/// spawn and reads it back after, so it has to appear as an effect of the spawn rather than as
/// something the test seeded beforehand.
pub(crate) fn record_successful_precombine_outputs(
    space: &InMemoryFileSpace,
    config: &ProjectConfig,
    ck_log: &Path,
) {
    space.add_file(config.fo4edit_data_dir().join("CombinedObjects.esp"));
    // Nested, because that is the shape Creation Kit writes precombines in.
    space.add_file(config.precombined_dir().join("cell").join("mesh.nif"));

    // Only a Clean-mode run emits the geometry PSG; a Filtered one never does.
    if config.build_mode == BuildMode::Clean {
        space.add_file(
            config
                .fo4edit_data_dir()
                .join(format!("{} - Geometry.psg", config.plugin.base_name)),
        );
    }

    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}
