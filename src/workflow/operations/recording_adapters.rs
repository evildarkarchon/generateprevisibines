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

use std::cell::RefCell;
use std::path::Path;

use crate::config::ProjectConfig;
use crate::error::Result;
use crate::files::InMemoryFileSpace;

use super::{Confirmation, Prompts};

/// A quiet Creation Kit log: present, readable, and free of the handle-array marker.
pub(crate) const QUIET_CK_LOG: &str = "Masterfile: Fallout4.esm\n";

/// Test [`Prompts`] that record the confirmations they were asked and answer from a script.
///
/// The script is one answer per [`Confirmation`] variant, so a test that refuses one question
/// still consents to every other — a refusal pins exactly the branch it names.
#[derive(Debug)]
pub(crate) struct RecordingPrompts {
    asked: RefCell<Vec<Confirmation>>,
    clear_precombined_answer: bool,
    clear_vis_answer: bool,
}

impl Default for RecordingPrompts {
    fn default() -> Self {
        Self {
            asked: RefCell::new(Vec::new()),
            // Consent by default, so a refusal is something a test has to ask for by name.
            clear_precombined_answer: true,
            clear_vis_answer: true,
        }
    }
}

impl RecordingPrompts {
    /// Prompts that have been asked nothing yet and consent to what they are asked.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Answer [`Confirmation::ClearPrecombined`] with a refusal instead of consent.
    #[must_use]
    pub(crate) const fn refusing_clear_precombined(mut self) -> Self {
        self.clear_precombined_answer = false;
        self
    }

    /// Answer [`Confirmation::ClearVis`] with a refusal instead of consent.
    #[must_use]
    pub(crate) const fn refusing_clear_vis(mut self) -> Self {
        self.clear_vis_answer = false;
        self
    }

    /// Every confirmation asked so far, in the order it was asked.
    #[must_use]
    pub(crate) fn asked(&self) -> Vec<Confirmation> {
        self.asked.borrow().clone()
    }
}

impl Prompts for RecordingPrompts {
    fn confirm(&self, confirmation: &Confirmation) -> Result<bool> {
        self.asked.borrow_mut().push(confirmation.clone());
        Ok(match confirmation {
            Confirmation::ClearPrecombined(_) => self.clear_precombined_answer,
            Confirmation::ClearVis(_) => self.clear_vis_answer,
        })
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

    // Only a `clean all` run (Clean, and Xbox since V2.99) emits the geometry PSG; a
    // Filtered one never does.
    if config.build_mode.is_clean_build() {
        space.add_file(
            config
                .fo4edit_data_dir()
                .join(format!("{} - Geometry.psg", config.plugin.base_name)),
        );
    }

    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}

/// Record what a successful Creation Kit `CompressPSG` run leaves in a Workflow Run's space.
///
/// The `.csg` beside the `.psg` it was compressed from, plus this run's log. The `.psg` is left
/// alone: Creation Kit does not remove it, the Compress PSG Operation does (batch 301), and a
/// simulation that removed it would hide whether the operation still does.
///
/// Like [`record_successful_precombine_outputs`], meant as an effects callback body, so the
/// `.csg` appears *because of* the spawn — which is what lets a test seed a stale one first
/// and still tell it apart from a fresh one.
pub(crate) fn record_successful_compress_outputs(
    space: &InMemoryFileSpace,
    config: &ProjectConfig,
    ck_log: &Path,
) {
    space.add_file(
        config
            .fo4edit_data_dir()
            .join(format!("{} - Geometry.csg", config.plugin.base_name)),
    );
    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}

/// Record what a successful Creation Kit `BuildCDX` run leaves in a Workflow Run's space.
///
/// `Data\<base name>.cdx`, plus this run's log. Like the other helpers here, meant as an
/// effects callback body, so the `.cdx` appears *because of* the spawn and a stale one seeded
/// beforehand can be told apart from it.
pub(crate) fn record_successful_cdx_outputs(
    space: &InMemoryFileSpace,
    config: &ProjectConfig,
    ck_log: &Path,
) {
    space.add_file(
        config
            .fo4edit_data_dir()
            .join(format!("{}.cdx", config.plugin.base_name)),
    );
    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}

/// Record what a successful Creation Kit `GeneratePreVisData` run leaves in a Workflow Run's
/// space.
///
/// `Data\Previs.esp`, one cluster's `.uvd` under `Data\vis`, and this run's quiet log. Like the
/// other helpers here, meant as an effects callback body, so `Previs.esp` appears *because of*
/// the spawn and a stale one seeded beforehand can be told apart from it. A test that needs a
/// different log overwrites it after calling this.
pub(crate) fn record_successful_previs_outputs(
    space: &InMemoryFileSpace,
    config: &ProjectConfig,
    ck_log: &Path,
) {
    space.add_file(config.fo4edit_data_dir().join("Previs.esp"));
    space.add_file(config.vis_dir().join("cluster.uvd"));
    space.add_file_with_contents(ck_log, QUIET_CK_LOG);
}
