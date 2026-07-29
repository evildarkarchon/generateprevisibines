---
type: "query"
date: "2026-07-28T23:47:28.379227+00:00"
question: "How are the Workflow Plan and dry-run preview built from registered Workflow Operations, including ordering, resume behavior, StepNotImplemented, and production operation availability?"
contributor: "graphify"
outcome: "useful"
source_nodes: ["WorkflowPlan", "ProductionOperationSource", "OperationCapability", "WorkflowRun"]
---

# Q: How are the Workflow Plan and dry-run preview built from registered Workflow Operations, including ordering, resume behavior, StepNotImplemented, and production operation availability?

## Answer

Expanded from original query via graph vocabulary: workflow, plan, operation, production, dry, run, resume, capability, registered, step, prepare, runnable. WorkflowPlan derives canonical mode and resume order, then takes the contiguous runnable prefix from registered operations; both real preparation and dry-run use the production constructor, and compatibility planning now preserves the same stop-at-first-unavailable invariant.

## Outcome

- Signal: useful

## Source Nodes

- WorkflowPlan
- ProductionOperationSource
- OperationCapability
- WorkflowRun