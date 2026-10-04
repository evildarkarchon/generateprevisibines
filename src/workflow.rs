//! Workflow state machine for the 8-step batch process.

pub mod operations;

use crate::config::{BuildMode, WorkflowStep};
use crate::error::{Error, Result};

/// Ordered planned and runnable steps for a Workflow Run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowPlan {
    planned_steps: Vec<WorkflowStep>,
    runnable_steps: Vec<WorkflowStep>,
}

impl WorkflowPlan {
    /// Resolve a Workflow Plan against operation availability supplied by the caller.
    ///
    /// Workflow Plan retains build-mode inclusion, resume sequencing, canonical ordering, and
    /// contiguous runnability without depending on any production registration type. Returns
    /// [`Error::StepNotImplemented`] for an unavailable requested resume step or empty runnable
    /// plan.
    fn resolve(
        build_mode: BuildMode,
        resume_from: Option<WorkflowStep>,
        operation_is_available: impl Fn(WorkflowStep) -> bool,
    ) -> Result<Self> {
        let planned_steps = Self::steps_for(build_mode, resume_from);
        let runnable_steps: Vec<_> = planned_steps
            .iter()
            .copied()
            .take_while(|step| operation_is_available(*step))
            .collect();

        if let Some(resume) = resume_from
            && !operation_is_available(resume)
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

    /// Steps this Workflow Run can execute with registered Workflow Operations.
    #[must_use]
    pub fn runnable_steps(&self) -> &[WorkflowStep] {
        &self.runnable_steps
    }

    /// Number of planned steps skipped because their Workflow Operations are unavailable.
    #[must_use]
    pub fn skipped_unrunnable_count(&self) -> usize {
        self.planned_steps.len() - self.runnable_steps.len()
    }

    /// Whether every planned step has a registered Workflow Operation, so a run that completes
    /// has built every Patch File and earns Finish.
    ///
    /// False on a partial rollout, which stops short of its plan: announcing that run complete,
    /// or deleting its Working Files, would claim a build that never happened.
    #[must_use]
    pub fn runs_every_planned_step(&self) -> bool {
        self.skipped_unrunnable_count() == 0
    }
}

/// Name a list of Workflow Steps for the console: `Step 4`, `Steps 4 and 5`, `Steps 1, 2 and 3`.
///
/// Steps are named in the order given; an empty list reads as `no steps`.
#[must_use]
pub fn describe_steps(steps: &[WorkflowStep]) -> String {
    let numbers: Vec<String> = steps.iter().map(|step| step.number().to_string()).collect();
    match numbers.as_slice() {
        [] => "no steps".to_string(),
        [only] => format!("Step {only}"),
        [leading @ .., last] => format!("Steps {} and {last}", leading.join(", ")),
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

    /// V2.99 Xbox is a clean build: every batch `filtered` test (219, 296, 305) is "Filtered vs.
    /// every other mode", so Xbox plans Steps 4 and 5 exactly as Clean does.
    #[test]
    fn xbox_mode_preserves_its_canonical_membership_and_order() {
        let steps = WorkflowPlan::steps_for(BuildMode::Xbox, None);

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
    fn a_plan_with_every_step_registered_runs_every_planned_step() {
        for build_mode in [BuildMode::Clean, BuildMode::Filtered, BuildMode::Xbox] {
            let plan = WorkflowPlan::resolve(build_mode, None, |_| true).unwrap();

            assert!(plan.runs_every_planned_step(), "build mode: {build_mode:?}");
        }
    }

    #[test]
    fn a_plan_with_an_unregistered_step_does_not_run_every_planned_step() {
        let plan = WorkflowPlan::resolve(BuildMode::Clean, None, |step| {
            step != WorkflowStep::AddPrevisToArchive
        })
        .unwrap();

        assert!(!plan.runs_every_planned_step());
    }

    #[test]
    fn a_step_list_names_one_step_in_the_singular() {
        assert_eq!(describe_steps(&[WorkflowStep::CompressPsg]), "Step 4");
    }

    #[test]
    fn a_step_list_names_several_steps_in_order() {
        assert_eq!(
            describe_steps(&[WorkflowStep::CompressPsg, WorkflowStep::BuildCdx]),
            "Steps 4 and 5"
        );
        assert_eq!(
            describe_steps(&[
                WorkflowStep::GeneratePrecombines,
                WorkflowStep::MergePrecombineObjects,
                WorkflowStep::CreateBa2FromPrecombines,
            ]),
            "Steps 1, 2 and 3"
        );
    }

    #[test]
    fn an_empty_step_list_says_so() {
        assert_eq!(describe_steps(&[]), "no steps");
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
