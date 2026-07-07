//! Workflow state machine for the 8-step batch process.

pub mod operations;

use crate::config::{BuildMode, WorkflowStep};
use crate::error::{Error, Result};

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

    /// Return the subset of planned steps this capability can execute.
    #[must_use]
    fn filter_steps(self, steps: &[WorkflowStep]) -> Vec<WorkflowStep> {
        steps
            .iter()
            .copied()
            .filter(|step| self.can_run(*step))
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
    /// Build the Workflow Plan for a build mode, optional resume step, and operation capability.
    ///
    /// Returns [`Error::StepNotImplemented`] when the requested resume step is not
    /// runnable by the capability, or when none of the planned steps can run.
    pub fn new(
        build_mode: BuildMode,
        resume_from: Option<WorkflowStep>,
        capability: OperationCapability,
    ) -> Result<Self> {
        let planned_steps = Self::steps_for(build_mode, resume_from);
        let runnable_steps = capability.filter_steps(&planned_steps);

        if let Some(resume) = resume_from {
            if !capability.can_run(resume) {
                return Err(Error::StepNotImplemented(resume.number()));
            }
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
    fn all_steps_capability() -> OperationCapability {
        OperationCapability::new(WorkflowStep::steps_for_mode(BuildMode::Clean))
    }

    #[test]
    fn clean_mode_includes_eight_steps() {
        let plan = WorkflowPlan::new(BuildMode::Clean, None, all_steps_capability()).unwrap();
        assert_eq!(plan.planned_steps().len(), 8);
    }

    #[test]
    fn filtered_mode_skips_psg_and_cdx() {
        let plan = WorkflowPlan::new(BuildMode::Filtered, None, all_steps_capability()).unwrap();
        let steps = plan.planned_steps();

        assert_eq!(steps.len(), 6);
        assert!(!steps.contains(&WorkflowStep::CompressPsg));
        assert!(!steps.contains(&WorkflowStep::BuildCdx));
    }

    #[test]
    fn resume_from_step_filters_earlier_steps() {
        let plan = WorkflowPlan::new(
            BuildMode::Clean,
            Some(WorkflowStep::GeneratePrevis),
            all_steps_capability(),
        )
        .unwrap();
        let steps = plan.planned_steps();

        assert_eq!(steps.first(), Some(&WorkflowStep::GeneratePrevis));
        assert!(!steps.contains(&WorkflowStep::GeneratePrecombines));
    }

    #[test]
    fn capability_filters_runnable_steps() {
        let capability = OperationCapability::new(&[WorkflowStep::GeneratePrecombines]);
        let plan = WorkflowPlan::new(BuildMode::Clean, None, capability).unwrap();

        assert_eq!(plan.runnable_steps(), &[WorkflowStep::GeneratePrecombines]);
        assert_eq!(plan.capability(), capability);
        assert_eq!(plan.skipped_unrunnable_count(), 7);
        assert!(plan.is_partial_due_to_capability());
    }

    #[test]
    fn resume_to_unrunnable_step_keeps_current_error() {
        let capability = OperationCapability::new(&[WorkflowStep::GeneratePrecombines]);
        let err = WorkflowPlan::new(
            BuildMode::Clean,
            Some(WorkflowStep::GeneratePrevis),
            capability,
        )
        .unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(6)));
    }

    #[test]
    fn resume_to_mode_skipped_step_reports_requested_step() {
        let capability = OperationCapability::new(&[WorkflowStep::GeneratePrecombines]);
        let err = WorkflowPlan::new(
            BuildMode::Filtered,
            Some(WorkflowStep::CompressPsg),
            capability,
        )
        .unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(4)));
    }

    #[test]
    fn no_runnable_steps_errors_on_first_planned_step() {
        let capability = OperationCapability::new(&[]);
        let err = WorkflowPlan::new(
            BuildMode::Clean,
            Some(WorkflowStep::GeneratePrevis),
            capability,
        )
        .unwrap_err();

        assert!(matches!(err, Error::StepNotImplemented(6)));
    }
}
