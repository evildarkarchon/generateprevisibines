//! Workflow state machine for the 8-step batch process.

pub mod operations;

use crate::config::{BuildMode, WorkflowStep};
use crate::error::{Error, Result};
use crate::workflow::operations::{
    ProductionOperationAdapters, WorkflowOperationExecutor, production_operation_source,
};

/// Plain description of which workflow steps the operation executor can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationCapability {
    runnable_steps: &'static [WorkflowStep],
}

impl OperationCapability {
    /// Create an operation capability from the fixed steps supported by the executor.
    ///
    /// The slice is static because operation capability is implementation-level behavior, not
    /// per-run state. Use [`WorkflowPlan`] to filter a specific Workflow Run.
    #[must_use]
    pub const fn new(runnable_steps: &'static [WorkflowStep]) -> Self {
        Self { runnable_steps }
    }

    /// Steps the operation executor can execute.
    #[must_use]
    pub const fn runnable_steps(self) -> &'static [WorkflowStep] {
        self.runnable_steps
    }

    /// Return the runnable prefix of planned steps this capability can execute.
    ///
    /// Runnability stops at the first unavailable operation so compatibility planning preserves
    /// the same dependency ordering as the production Workflow Operation source.
    #[must_use]
    fn filter_steps(self, steps: &[WorkflowStep]) -> Vec<WorkflowStep> {
        steps
            .iter()
            .copied()
            .take_while(|step| self.can_run(*step))
            .collect()
    }

    fn can_run(self, step: WorkflowStep) -> bool {
        self.runnable_steps.contains(&step)
    }
}

/// Ordered steps for a Workflow Run after build mode, resume, and capability are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowPlan {
    planned_steps: Vec<WorkflowStep>,
    runnable_steps: Vec<WorkflowStep>,
    capability: OperationCapability,
}

impl WorkflowPlan {
    /// Build the production Workflow Plan for a build mode and optional resume step.
    ///
    /// Returns [`Error::StepNotImplemented`] when the requested resume step is not
    /// registered as a production Workflow Operation, or when none of the planned steps are
    /// currently runnable.
    pub fn new(build_mode: BuildMode, resume_from: Option<WorkflowStep>) -> Result<Self> {
        let planned_steps = Self::steps_for(build_mode, resume_from);
        let operations = production_operation_source();
        let runnable_steps = operations.filter_steps(&planned_steps);
        let capability =
            WorkflowOperationExecutor::<ProductionOperationAdapters>::production_capability();

        Self::from_filtered_steps(
            planned_steps,
            runnable_steps,
            resume_from,
            resume_from.is_none_or(|resume| operations.contains(resume)),
            capability,
        )
    }

    /// Build a Workflow Plan with synthetic capability for the temporary Workflow Run migration.
    ///
    /// Ticket #8 removes this compatibility constructor after Workflow Run execution no longer
    /// stores capability separately from the production Workflow Operation source. Returns
    /// [`Error::StepNotImplemented`] when the requested resume step or entire filtered plan is not
    /// runnable under `capability`.
    pub(crate) fn new_with_capability(
        build_mode: BuildMode,
        resume_from: Option<WorkflowStep>,
        capability: OperationCapability,
    ) -> Result<Self> {
        let planned_steps = Self::steps_for(build_mode, resume_from);
        let runnable_steps = capability.filter_steps(&planned_steps);

        Self::from_filtered_steps(
            planned_steps,
            runnable_steps,
            resume_from,
            resume_from.is_none_or(|resume| capability.can_run(resume)),
            capability,
        )
    }

    /// Validate filtered production or compatibility steps and assemble the immutable plan.
    ///
    /// `resume_is_runnable` must report membership in the same operation source or compatibility
    /// capability that produced `runnable_steps`. Returns [`Error::StepNotImplemented`] for an
    /// unavailable requested resume step or an empty runnable plan.
    fn from_filtered_steps(
        planned_steps: Vec<WorkflowStep>,
        runnable_steps: Vec<WorkflowStep>,
        resume_from: Option<WorkflowStep>,
        resume_is_runnable: bool,
        capability: OperationCapability,
    ) -> Result<Self> {
        if let Some(resume) = resume_from
            && !resume_is_runnable
        {
            return Err(Error::StepNotImplemented(resume.number()));
        }

        if runnable_steps.is_empty() {
            return Err(Error::StepNotImplemented(
                planned_steps.first().map_or(1, |step| step.number()),
            ));
        }

        Ok(Self {
            planned_steps,
            runnable_steps,
            capability,
        })
    }

    /// Steps this Workflow Run will attempt, honoring build mode and resume.
    #[must_use]
    pub fn steps_for(
        build_mode: BuildMode,
        resume_from: Option<WorkflowStep>,
    ) -> Vec<WorkflowStep> {
        let all = WorkflowStep::steps_for_mode(build_mode);
        let Some(resume) = resume_from else {
            return all.to_vec();
        };

        all.iter().copied().filter(|step| *step >= resume).collect()
    }

    /// Steps belonging to this Workflow Run, including currently unrunnable steps.
    #[must_use]
    pub fn planned_steps(&self) -> &[WorkflowStep] {
        &self.planned_steps
    }

    /// Steps this Workflow Run can execute with the selected operation capability.
    #[must_use]
    pub fn runnable_steps(&self) -> &[WorkflowStep] {
        &self.runnable_steps
    }

    /// Operation capability used to derive this Workflow Plan's runnable steps.
    #[must_use]
    pub const fn capability(&self) -> OperationCapability {
        self.capability
    }

    /// Number of planned steps skipped because the operation executor cannot run them yet.
    #[must_use]
    pub fn skipped_unrunnable_count(&self) -> usize {
        self.planned_steps.len() - self.runnable_steps.len()
    }

    /// Whether this plan is partial because the operation executor lacks later-step capability.
    #[must_use]
    pub fn is_partial_due_to_capability(&self) -> bool {
        self.skipped_unrunnable_count() > 0
    }
}

/// Print the resume menu labels (batch `:GetStep`).
pub fn print_resume_menu(build_mode: BuildMode) {
    println!();
    for step in WorkflowStep::steps_for_mode(build_mode) {
        println!("[{}] {}", step.number(), step.label());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_mode_preserves_all_steps_in_canonical_order() {
        let steps = WorkflowPlan::steps_for(BuildMode::Clean, None);

        assert_eq!(
            steps,
            [
                WorkflowStep::GeneratePrecombines,
                WorkflowStep::MergePrecombineObjects,
                WorkflowStep::CreateBa2FromPrecombines,
                WorkflowStep::CompressPsg,
                WorkflowStep::BuildCdx,
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
            ]
        );
    }

    #[test]
    fn filtered_mode_preserves_its_canonical_membership_and_order() {
        let steps = WorkflowPlan::steps_for(BuildMode::Filtered, None);

        assert_eq!(
            steps,
            [
                WorkflowStep::GeneratePrecombines,
                WorkflowStep::MergePrecombineObjects,
                WorkflowStep::CreateBa2FromPrecombines,
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
            ]
        );
    }

    #[test]
    fn xbox_mode_preserves_its_canonical_membership_and_order() {
        let steps = WorkflowPlan::steps_for(BuildMode::Xbox, None);

        assert_eq!(
            steps,
            [
                WorkflowStep::GeneratePrecombines,
                WorkflowStep::MergePrecombineObjects,
                WorkflowStep::CreateBa2FromPrecombines,
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
            ]
        );
    }

    #[test]
    fn resume_preserves_the_canonical_suffix() {
        let steps = WorkflowPlan::steps_for(BuildMode::Clean, Some(WorkflowStep::GeneratePrevis));

        assert_eq!(
            steps,
            [
                WorkflowStep::GeneratePrevis,
                WorkflowStep::MergePrevis,
                WorkflowStep::AddPrevisToArchive,
            ]
        );
    }
}
