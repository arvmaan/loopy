//! Loopy composable pipeline — building blocks.
//!
//! This is the foundation for evolving Loopy from a single hardcoded pipeline
//! into a plug-and-play looping engine for any task (see `docs/LOOPY-VISION.md`).
//!
//! A [`StageBlock`] is a reusable building block describing ONE stage: how to
//! prompt the agent, what it consumes/produces, how it gates (pass / auto-fix /
//! escalate a finding — the no-mistakes model), and whether it pauses for human
//! review. A [`PipelineTemplate`] is an ordered list of blocks. Today's Loopy
//! flow ("build a presentation-ready POC from a design doc") is registered as one
//! built-in template; new task types (coding task, bug fix, …) are just more
//! templates.
//!
//! This module is intentionally ADDITIVE and behavior-free: it describes
//! pipelines as data. The existing `engine.rs` state machine is unchanged and
//! remains the execution authority. A later step wires the engine to consume
//! these templates; for now this gives us the vocabulary and the registry, with
//! the POC template proven to match the live pipeline.

use serde::{Deserialize, Serialize};

/// What kind of work a stage performs. Maps loosely onto the agent role / hat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    /// Research / analyze the task and its environment (Loopy: Scan).
    Scan,
    /// Gather external knowledge / strategies / prior art (broader than Scan,
    /// which is codebase-focused).
    Research,
    /// Decompose into a plan / work breakdown (Loopy: Plan → tracks).
    Plan,
    /// Reproduce + root-cause a bug before fixing it.
    Diagnose,
    /// Produce a design / spec document for the task.
    Design,
    /// Produce UI wireframes / mockups.
    UiMocks,
    /// Implement the change — may fan out into parallel sub-units (Loopy: Tracks).
    Implement,
    /// Refactor existing code for simplicity, then verify behavior is unchanged.
    Simplify,
    /// Profile / measure performance, optimize, and judge the result.
    Benchmark,
    /// Human team-review checkpoint — gather feedback before proceeding.
    TeamReview,
    /// Review the produced changes (standard code review).
    Review,
    /// Adversarial review — a skeptic agent that tries to refute/break the change.
    AdversarialReview,
    /// Security-lens review pass over the change.
    SecurityReview,
    /// Run automated tests / lint / build gates.
    Verify,
    /// Update documentation / runbooks for the change.
    Document,
    /// Deploy to a beta/test environment and exercise it (Loopy: Test Flight).
    BetaTest,
    /// Land / submit the change (Loopy: Land; coding task: submit CR).
    Submit,
}

impl StageKind {
    /// All block kinds the user can add from the palette, in a sensible order.
    pub fn all() -> [StageKind; 17] {
        [
            StageKind::Scan, StageKind::Research, StageKind::Plan, StageKind::Diagnose,
            StageKind::Design, StageKind::UiMocks, StageKind::Implement, StageKind::Simplify,
            StageKind::Benchmark, StageKind::TeamReview, StageKind::Review,
            StageKind::AdversarialReview, StageKind::SecurityReview, StageKind::Verify,
            StageKind::Document, StageKind::BetaTest, StageKind::Submit,
        ]
    }

    /// The canonical block id for this kind (matches what plan_blocks emits).
    pub fn id(self) -> &'static str {
        match self {
            StageKind::Scan => "scan",
            StageKind::Research => "research",
            StageKind::Plan => "plan",
            StageKind::Diagnose => "diagnose",
            StageKind::Design => "design",
            StageKind::UiMocks => "ui_mocks",
            StageKind::Implement => "implement",
            StageKind::Simplify => "simplify",
            StageKind::Benchmark => "benchmark",
            StageKind::TeamReview => "team_review",
            StageKind::Review => "review",
            StageKind::AdversarialReview => "adversarial_review",
            StageKind::SecurityReview => "security_review",
            StageKind::Verify => "verify",
            StageKind::Document => "document",
            StageKind::BetaTest => "beta_test",
            StageKind::Submit => "submit",
        }
    }

    /// Parse a kind from its snake_case string (as sent by the UI / API).
    pub fn from_id(s: &str) -> Option<StageKind> {
        Some(match s {
            "scan" => StageKind::Scan,
            "research" => StageKind::Research,
            "plan" => StageKind::Plan,
            "diagnose" => StageKind::Diagnose,
            "design" => StageKind::Design,
            "ui_mocks" => StageKind::UiMocks,
            "implement" | "tracks" => StageKind::Implement,
            "simplify" => StageKind::Simplify,
            "benchmark" => StageKind::Benchmark,
            "team_review" => StageKind::TeamReview,
            "review" => StageKind::Review,
            "adversarial_review" => StageKind::AdversarialReview,
            "security_review" => StageKind::SecurityReview,
            "verify" => StageKind::Verify,
            "document" => StageKind::Document,
            "beta_test" => StageKind::BetaTest,
            "submit" | "land" => StageKind::Submit,
            _ => return None,
        })
    }

    /// Flight-themed display name for the UI.
    pub fn display_name(self) -> &'static str {
        match self {
            StageKind::Scan => "Recon",
            StageKind::Research => "Intel",
            StageKind::Plan => "Flight Plan",
            StageKind::Diagnose => "Black Box",
            StageKind::Design => "Blueprint",
            StageKind::UiMocks => "Mockups",
            StageKind::Implement => "Build",
            StageKind::Simplify => "Trim",
            StageKind::Benchmark => "Wind Tunnel",
            StageKind::TeamReview => "Crew Review",
            StageKind::Review => "Debrief",
            StageKind::AdversarialReview => "Red Team",
            StageKind::SecurityReview => "Threat Scan",
            StageKind::Verify => "Preflight",
            StageKind::Document => "Logbook",
            StageKind::BetaTest => "Test Flight",
            StageKind::Submit => "Land",
        }
    }

    /// Category for grouping blocks in the UI block library.
    pub fn category(self) -> &'static str {
        match self {
            StageKind::Scan | StageKind::Research | StageKind::Plan | StageKind::Diagnose => "Understand",
            StageKind::Design | StageKind::UiMocks => "Design",
            StageKind::Implement | StageKind::Simplify | StageKind::Benchmark => "Build",
            StageKind::TeamReview | StageKind::Review | StageKind::AdversarialReview
            | StageKind::SecurityReview | StageKind::Verify => "Review & Verify",
            StageKind::Document | StageKind::BetaTest | StageKind::Submit => "Ship",
        }
    }

    /// One-line description shown when hovering the block's badge in the UI.
    pub fn description(self) -> &'static str {
        match self {
            StageKind::Scan => "Research the codebase and environment; map what exists and what's needed.",
            StageKind::Research => "Gather external knowledge, prior art, and strategies (beyond the codebase).",
            StageKind::Plan => "Break the task into a concrete work breakdown / tracks.",
            StageKind::Diagnose => "Reproduce the issue and find its root cause before fixing.",
            StageKind::Design => "Produce a design / spec document before building.",
            StageKind::UiMocks => "Produce UI wireframes / mockups for the change.",
            StageKind::Implement => "Do the actual implementation work (may fan out into tracks).",
            StageKind::Simplify => "Refactor for simplicity, then verify behavior is unchanged.",
            StageKind::Benchmark => "Profile and measure performance, optimize, and judge the result.",
            StageKind::TeamReview => "Pause for the human team to review and give feedback.",
            StageKind::Review => "Standard code review of the produced changes.",
            StageKind::AdversarialReview => "A skeptic agent that tries to refute and break the change to surface real problems.",
            StageKind::SecurityReview => "A security-lens pass over the change (auth, data handling, injection, secrets).",
            StageKind::Verify => "Run automated tests, lint, and build gates; auto-fix mechanical issues.",
            StageKind::Document => "Update documentation and runbooks for the change.",
            StageKind::BetaTest => "Deploy to a beta/test environment and run E2E checks. Never touches production.",
            StageKind::Submit => "Open the CR/PR and land the work.",
        }
    }

    /// A default checkpoint for a block kind when the user adds it ad-hoc — so a
    /// hand-built pipeline still pauses for review where it sensibly should.
    pub fn default_checkpoint(self) -> Option<(CheckpointKind, RerouteTarget)> {
        match self {
            StageKind::Plan => Some((CheckpointKind::Plan, RerouteTarget::Plan)),
            StageKind::Design => Some((CheckpointKind::Results, RerouteTarget::SelfStage)),
            StageKind::UiMocks => Some((CheckpointKind::Results, RerouteTarget::SelfStage)),
            StageKind::TeamReview => Some((CheckpointKind::Results, RerouteTarget::SelfStage)),
            StageKind::Review
            | StageKind::AdversarialReview
            | StageKind::SecurityReview => Some((CheckpointKind::Code, RerouteTarget::Implement)),
            _ => None,
        }
    }

    /// The agent hat collection this kind of work uses, or `None` for plain
    /// (traditional-mode) agents. Mirrors today's `hat_collection_for_stage`:
    /// implementation work uses the code-assist hat; research/review/submit
    /// stages run without a hat.
    pub fn default_hat(self) -> Option<&'static str> {
        match self {
            // Implementation and gates that touch code use the code-assist hat.
            StageKind::Implement
            | StageKind::Simplify
            | StageKind::Benchmark
            | StageKind::Verify => Some("builtin:code-assist"),
            StageKind::Scan
            | StageKind::Research
            | StageKind::Plan
            | StageKind::Diagnose
            | StageKind::Design
            | StageKind::UiMocks
            | StageKind::TeamReview
            | StageKind::Review
            | StageKind::AdversarialReview
            | StageKind::SecurityReview
            | StageKind::Document
            | StageKind::BetaTest
            | StageKind::Submit => None,
        }
    }

    /// The completion-promise topic a generic agent for this stage emits when
    /// done. Used to build `ralph.yml` for blocks the same way the engine's
    /// per-stage promises work today.
    pub fn completion_promise(self) -> &'static str {
        match self {
            StageKind::Scan => "scan.complete",
            StageKind::Research => "research.complete",
            StageKind::Plan => "plan.complete",
            StageKind::Diagnose => "diagnose.complete",
            StageKind::Design => "design.complete",
            StageKind::UiMocks => "ui_mocks.complete",
            StageKind::Implement => "implement.complete",
            StageKind::Simplify => "simplify.complete",
            StageKind::Benchmark => "benchmark.complete",
            StageKind::TeamReview => "team_review.complete",
            StageKind::Review => "review.complete",
            StageKind::AdversarialReview => "adversarial_review.complete",
            StageKind::SecurityReview => "security_review.complete",
            StageKind::Verify => "verify.complete",
            StageKind::Document => "document.complete",
            StageKind::BetaTest => "beta_test.complete",
            StageKind::Submit => "submit.complete",
        }
    }

    /// A short, reusable role line for the stage's prompt. The full prompt is
    /// still composed elsewhere (today: orchestrator `build_stage_prompt`); this
    /// is the per-kind seed a generic prompt builder can use.
    pub fn role_line(self) -> &'static str {
        match self {
            StageKind::Scan => "You are a technical researcher analyzing a task and its environment.",
            StageKind::Research => "You are a researcher gathering external knowledge, prior art, and candidate strategies for the task.",
            StageKind::Plan => "You are a planner decomposing the task into a clear work breakdown.",
            StageKind::Diagnose => "You are debugging: reproduce the issue and identify its root cause before any fix.",
            StageKind::Design => "You are a designer producing a clear design/spec document for the task.",
            StageKind::UiMocks => "You are a UI designer producing wireframes / mockups for the change.",
            StageKind::Implement => "You are an engineer implementing the change end to end.",
            StageKind::Simplify => "You are refactoring for simplicity. Reduce complexity without changing behavior; verify nothing breaks.",
            StageKind::Benchmark => "You are profiling and optimizing performance, then judging the measured result.",
            StageKind::TeamReview => "Summarize the work so far for a human team review and surface open questions.",
            StageKind::Review => "You are a critical reviewer. Try to find real problems with the change.",
            StageKind::AdversarialReview => "You are an adversarial reviewer. Actively try to REFUTE and break the change — find bugs, edge cases, and unsound assumptions. Default to skeptical.",
            StageKind::SecurityReview => "You are a security reviewer. Inspect the change for auth flaws, injection, unsafe data handling, and leaked secrets.",
            StageKind::Verify => "You are running automated tests, lint, and build gates; auto-fix mechanical issues.",
            StageKind::Document => "You are updating documentation and runbooks to reflect the change.",
            StageKind::BetaTest => "You are running the change against a beta/test environment and reporting results. Never touch production.",
            StageKind::Submit => "You are finalizing and submitting the change (e.g. opening a CR/PR).",
        }
    }
}

/// How a stage gates progression — the no-mistakes contract. A stage either
/// passes, applies safe mechanical fixes itself, or escalates a finding for the
/// human to resolve. Nothing advances past a non-passing gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatePolicy {
    /// Advance as soon as the stage completes; no gating.
    Pass,
    /// Apply safe, mechanical fixes automatically, then advance.
    AutoFix,
    /// Surface findings; advance only once they are resolved/approved.
    EscalateFinding,
}

/// Whether a stage pauses for explicit human review before advancing, and what
/// kind of review surface it presents. `None` = fully autonomous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    /// Review a plan/spec document (Loopy: AwaitingPlanReview).
    Plan,
    /// Review produced code changes (Loopy: Flight Check).
    Code,
    /// Edit/approve a draft before a run (Loopy: Test Flight plan).
    EditableDraft,
    /// Review run results, accept or request changes (Loopy: Test Flight review).
    Results,
}

/// Where a rejected checkpoint routes feedback. Feedback is always additive —
/// the target stage re-runs on top of existing work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RerouteTarget {
    /// Re-run this same stage with the feedback appended.
    SelfStage,
    /// Re-run the implement stage(s) — Loopy's Flight Check / Test Flight reject.
    Implement,
    /// Re-run from planning.
    Plan,
}

/// A reusable pipeline building block describing one stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageBlock {
    /// Stable identifier, e.g. "scan", "implement", "adversarial_review".
    pub id: String,
    /// Human-facing label, e.g. "Flight Check".
    pub label: String,
    pub kind: StageKind,
    pub gate: GatePolicy,
    /// If set, the stage pauses for human review of this kind before advancing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<CheckpointKind>,
    /// Where a rejected checkpoint loops back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_reject: Option<RerouteTarget>,
    /// Whether the stage is opt-in (skipped unless the user enables it). Loopy's
    /// Test Flight is opt-in.
    #[serde(default)]
    pub optional: bool,
    /// Whether this block is fixed in the pipeline — the user cannot remove or
    /// reorder it. Pipelines always start with a locked Scan and end with a
    /// locked Land; everything between is freely editable.
    #[serde(default)]
    pub locked: bool,
}

impl StageBlock {
    /// A plain autonomous stage with no gate or checkpoint.
    pub fn simple(id: &str, label: &str, kind: StageKind) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            kind,
            gate: GatePolicy::Pass,
            checkpoint: None,
            on_reject: None,
            optional: false,
            locked: false,
        }
    }

    /// Mark this block as locked (fixed first/last — can't be moved or removed).
    pub fn locked(mut self) -> Self {
        self.locked = true;
        self
    }

    pub fn with_checkpoint(mut self, c: CheckpointKind, reject: RerouteTarget) -> Self {
        self.checkpoint = Some(c);
        self.on_reject = Some(reject);
        self
    }

    pub fn with_gate(mut self, gate: GatePolicy) -> Self {
        self.gate = gate;
        self
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// An ordered pipeline of building blocks, addressable by name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineTemplate {
    /// Stable id, e.g. "poc-from-design-doc", "coding-task".
    pub id: String,
    /// Human-facing name.
    pub name: String,
    /// One-line description shown in the preset picker.
    pub description: String,
    /// Whether the engine can actually EXECUTE this pipeline today. The POC
    /// pipeline is executable; other templates are described (for the building-
    /// block model + planner) but not yet wired into `transition()`. Guards
    /// against silently running the POC sequence when a different one was asked
    /// for. Flip to true as each template's execution is implemented.
    #[serde(default)]
    pub executable: bool,
    pub stages: Vec<StageBlock>,
}

impl PipelineTemplate {
    pub fn stage(&self, id: &str) -> Option<&StageBlock> {
        self.stages.iter().find(|s| s.id == id)
    }

    /// Position of a stage by id.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.stages.iter().position(|s| s.id == id)
    }

    /// The first stage of the pipeline.
    pub fn first_stage(&self) -> Option<&StageBlock> {
        self.stages.first()
    }

    /// Build a custom, executable pipeline from a user-edited block list. Each
    /// entry is (id, kind, optional label). Unknown kinds are skipped. Applies
    /// each kind's default checkpoint so a hand-built pipeline still pauses for
    /// review where sensible. This is what the corrected Loopy flow runs after
    /// the user edits the proposal.
    pub fn from_blocks(blocks: &[(String, String, Option<String>)]) -> PipelineTemplate {
        let stages: Vec<StageBlock> = blocks
            .iter()
            .filter_map(|(id, kind_str, label)| {
                let kind = StageKind::from_id(kind_str)?;
                let label = label.clone().unwrap_or_else(|| id.clone());
                let mut block = StageBlock::simple(id, &label, kind);
                if let Some((c, r)) = kind.default_checkpoint() {
                    block = block.with_checkpoint(c, r);
                }
                Some(block)
            })
            .collect();
        PipelineTemplate {
            id: "custom".into(),
            name: "Custom pipeline".into(),
            description: "User-defined block list.".into(),
            executable: true,
            stages,
        }
    }

    /// The stage that runs after `after_id`. When `include_optional` is false,
    /// optional stages (e.g. Test Flight) are skipped — modelling the user
    /// declining the opt-in. Returns `None` at the end of the pipeline.
    pub fn next_stage(&self, after_id: &str, include_optional: bool) -> Option<&StageBlock> {
        let idx = self.index_of(after_id)?;
        self.stages[idx + 1..]
            .iter()
            .find(|s| include_optional || !s.optional)
    }

    /// Whether `a_id` comes before `b_id` in the pipeline (both must exist).
    pub fn is_before(&self, a_id: &str, b_id: &str) -> bool {
        match (self.index_of(a_id), self.index_of(b_id)) {
            (Some(a), Some(b)) => a < b,
            _ => false,
        }
    }
}

/// Today's Loopy flow as a template. This is the regression baseline — the
/// "build a presentation-ready POC from a design doc" preset. It MUST keep
/// describing the live pipeline; if the engine's stages change, update this in
/// lockstep.
pub fn poc_from_design_doc() -> PipelineTemplate {
    PipelineTemplate {
        id: "poc-from-design-doc".into(),
        name: "POC from design doc".into(),
        description: "Take a design doc and deliver a working POC across packages.".into(),
        executable: true,
        stages: vec![
            StageBlock::simple("scan", "Scan", StageKind::Scan),
            StageBlock::simple("plan", "Plan", StageKind::Plan)
                .with_checkpoint(CheckpointKind::Plan, RerouteTarget::Plan),
            StageBlock::simple("tracks", "Tracks", StageKind::Implement),
            // Flight Check — review the committed changes.
            StageBlock::simple("flight_check", "Flight Check", StageKind::Review)
                .with_gate(GatePolicy::EscalateFinding)
                .with_checkpoint(CheckpointKind::Code, RerouteTarget::Implement),
            // Test Flight — opt-in beta deploy + E2E, two checkpoints.
            StageBlock::simple("test_flight", "Test Flight", StageKind::BetaTest)
                .with_gate(GatePolicy::EscalateFinding)
                .with_checkpoint(CheckpointKind::EditableDraft, RerouteTarget::Implement)
                .optional(),
            StageBlock::simple("land", "Land", StageKind::Submit),
        ],
    }
}

/// A coding-task template: implement → beta-test → adversarial review → submit CR.
/// (Not yet wired to the engine — registered so the building-block model has a
/// second concrete example, per the vision doc.)
pub fn coding_task() -> PipelineTemplate {
    PipelineTemplate {
        id: "coding-task".into(),
        name: "Coding task".into(),
        description: "Implement a coding task, beta-test it, adversarially review it, and submit a CR.".into(),
        // Executable via the generic linear driver (LinearRun + spawn_block +
        // checkpoint resume). Not the POC state machine.
        executable: true,
        stages: vec![
            StageBlock::simple("implement", "Implement", StageKind::Implement),
            StageBlock::simple("beta_test", "Beta test", StageKind::BetaTest)
                .with_gate(GatePolicy::EscalateFinding)
                .optional(),
            StageBlock::simple("adversarial_review", "Adversarial review", StageKind::Review)
                .with_gate(GatePolicy::EscalateFinding)
                .with_checkpoint(CheckpointKind::Code, RerouteTarget::Implement),
            StageBlock::simple("submit_cr", "Submit CR", StageKind::Submit),
        ],
    }
}

/// A design-task template demonstrating flexible block arrangement:
/// Scan → Plan → Design → Team Review → Tracks. Shows that blocks compose freely
/// (this is the arrangement the user called out). Executable via the linear driver.
pub fn design_task() -> PipelineTemplate {
    PipelineTemplate {
        id: "design-task".into(),
        name: "Design task".into(),
        description: "Scan, plan, produce a design, get team review, then implement.".into(),
        executable: true,
        stages: vec![
            StageBlock::simple("scan", "Scan", StageKind::Scan),
            StageBlock::simple("plan", "Plan", StageKind::Plan)
                .with_checkpoint(CheckpointKind::Plan, RerouteTarget::Plan),
            StageBlock::simple("design", "Design", StageKind::Design),
            StageBlock::simple("team_review", "Team review", StageKind::TeamReview)
                .with_gate(GatePolicy::EscalateFinding)
                .with_checkpoint(CheckpointKind::Results, RerouteTarget::SelfStage),
            StageBlock::simple("tracks", "Tracks", StageKind::Implement),
        ],
    }
}

/// Map an engine [`StageId`](crate::models::StageId) to the POC template's
/// stage block id. This is the bridge that keeps the building-block description
/// honest against the live engine; a sync test asserts they stay aligned.
pub fn poc_block_id_for_stage(stage: crate::models::StageId) -> Option<&'static str> {
    use crate::models::StageId;
    match stage {
        StageId::Scan => Some("scan"),
        StageId::Plan => Some("plan"),
        StageId::OrbitalLanes => Some("tracks"),
        StageId::TestFlight => Some("test_flight"),
        StageId::Land => Some("land"),
        // Idea is implicit (always complete at start); RequirementsAnalysis is
        // legacy/removed from the V2 flow — neither is a POC building block.
        StageId::Idea | StageId::RequirementsAnalysis => None,
    }
}

/// All built-in pipeline templates, in display order.
pub fn builtin_templates() -> Vec<PipelineTemplate> {
    vec![poc_from_design_doc(), coding_task(), design_task()]
}

/// Look up a template by id.
pub fn template_by_id(id: &str) -> Option<PipelineTemplate> {
    builtin_templates().into_iter().find(|t| t.id == id)
}

/// Result of planning: which template best fits a prompt, with a short rationale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanChoice {
    pub template_id: String,
    pub reason: String,
}

/// Heuristic planner: map a freeform task prompt to the best-fit built-in
/// template. This is the "identify which loop steps are needed" step of the
/// Loopy vision. Intentionally simple and deterministic for now (keyword/intent
/// scoring) — an LLM planner can later replace this behind the same signature.
///
/// Default/fallback is the POC pipeline, which is also the safe choice for
/// design-doc-shaped work.
pub fn plan_template_for_prompt(prompt: &str) -> PlanChoice {
    let p = prompt.to_lowercase();

    // Signals that this is a design-doc → POC delivery (the original Loopy job).
    const POC_SIGNALS: &[&str] = &[
        "design doc", "design document", "poc", "proof of concept",
        "deliver", "from the doc", "build out", "end to end feature",
        "multiple packages", "across packages",
    ];
    // Signals that this is a focused single coding task.
    const CODING_SIGNALS: &[&str] = &[
        "fix", "bug", "implement", "add a", "refactor", "rename", "update the",
        "write a", "change the", "submit a cr", "code review", "small change",
        "function", "method", "endpoint",
    ];

    let count = |signals: &[&str]| signals.iter().filter(|s| p.contains(**s)).count();
    let poc = count(POC_SIGNALS);
    let coding = count(CODING_SIGNALS);

    if coding > poc {
        PlanChoice {
            template_id: "coding-task".into(),
            reason: format!("Prompt looks like a focused coding task ({coding} coding signal(s) vs {poc} POC signal(s))."),
        }
    } else if poc > 0 {
        PlanChoice {
            template_id: "poc-from-design-doc".into(),
            reason: format!("Prompt looks like a design-doc → POC delivery ({poc} POC signal(s))."),
        }
    } else {
        PlanChoice {
            template_id: "poc-from-design-doc".into(),
            reason: "No strong signal; defaulting to the POC pipeline.".into(),
        }
    }
}

/// A proposed pipeline as an EDITABLE block list — the core of the corrected
/// Loopy flow. The planner suggests blocks for a task; the user can then add,
/// remove, or reorder them before execution. (Blocks, not a fixed template, are
/// the primitive.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedPipeline {
    /// Short rationale for the proposal.
    pub reason: String,
    /// The proposed ordered blocks — fully editable by the user.
    pub blocks: Vec<StageBlock>,
}

/// Build a ProposedPipeline from an ordered list of middle block-kind ids (as
/// chosen by the LLM planner), bookended by locked Scan first and Land last.
/// Each middle block gets its kind's default checkpoint. Unknown ids are skipped.
pub fn compose_from_kinds(kind_ids: &[String], reason: String) -> ProposedPipeline {
    let mut blocks = vec![StageBlock::simple("scan", "Scan", StageKind::Scan).locked()];
    for id in kind_ids {
        // Skip scan/submit — they are the locked bookends.
        if id == "scan" || id == "submit" || id == "land" {
            continue;
        }
        if let Some(kind) = StageKind::from_id(id) {
            let mut b = StageBlock::simple(kind.id(), kind.display_name(), kind);
            if let Some((c, r)) = kind.default_checkpoint() {
                b = b.with_checkpoint(c, r);
            }
            blocks.push(b);
        }
    }
    blocks.push(StageBlock::simple("land", "Land", StageKind::Submit).locked());
    ProposedPipeline { reason, blocks }
}

/// Propose an editable block list for a freeform task by COMPOSING the blocks
/// the task needs (not just picking a template). Each block kind is included if
/// the prompt signals a need for it; blocks are then emitted in canonical
/// pipeline order. This is the real "determine what blocks it needs" step — e.g.
/// a "design a feature and ship it to beta" task yields
/// scan → plan → design → team_review → implement → beta_test → submit.
///
/// Deterministic keyword heuristic for now (swappable for an LLM behind the same
/// signature). Always returns a sensible non-empty list the user can edit.
pub fn plan_blocks_for_task(prompt: &str) -> ProposedPipeline {
    let p = prompt.to_lowercase();
    let has = |sigs: &[&str]| sigs.iter().any(|s| p.contains(*s));

    let needs_design = has(&["design", "architecture", "spec", "rfc", "wireframe", "mockup", "ux", "diagram"]);
    let needs_team_review = has(&["team review", "design review", "review with", "get feedback", "stakeholder", "sign-off", "sign off"]);
    let needs_beta = has(&["beta", "deploy", "test environment", "e2e", "end to end test", "integration test", "staging", "smoke test"]);
    let needs_submit = has(&["cr", "pull request", " pr", "submit", "merge", "ship", "land", "release"]);
    // Focused single-change coding vs. broad multi-package delivery.
    let is_focused_coding = has(&["fix", "bug", "small change", "refactor", "rename", "tweak", "one-line", "single function"]);
    let is_delivery = has(&["poc", "proof of concept", "design doc", "deliver", "across packages", "multiple packages", "feature"]);

    // Compose blocks in canonical order, including only what's needed.
    let mut blocks: Vec<StageBlock> = Vec::new();
    let mut reasons: Vec<&str> = Vec::new();

    // Scan is a fixed first block — every pipeline starts here and it's locked.
    blocks.push(StageBlock::simple("scan", "Scan", StageKind::Scan).locked());
    reasons.push("scan");
    // Planning: for anything non-trivial.
    if is_delivery || needs_design || needs_team_review || !is_focused_coding {
        blocks.push(StageBlock::simple("plan", "Plan", StageKind::Plan)
            .with_checkpoint(CheckpointKind::Plan, RerouteTarget::Plan));
        reasons.push("plan");
    }
    if needs_design {
        blocks.push(StageBlock::simple("design", "Design", StageKind::Design));
        reasons.push("design");
    }
    if needs_team_review {
        blocks.push(StageBlock::simple("team_review", "Team review", StageKind::TeamReview)
            .with_gate(GatePolicy::EscalateFinding)
            .with_checkpoint(CheckpointKind::Results, RerouteTarget::SelfStage));
        reasons.push("team review");
    }
    // Implementation: essentially always (the work itself).
    blocks.push(StageBlock::simple("implement", "Implement", StageKind::Implement));
    reasons.push("implement");

    if needs_beta {
        blocks.push(StageBlock::simple("beta_test", "Beta test", StageKind::BetaTest)
            .with_gate(GatePolicy::EscalateFinding));
        reasons.push("beta test");
    }
    // Code review before submitting any real change.
    blocks.push(StageBlock::simple("review", "Review", StageKind::Review)
        .with_gate(GatePolicy::EscalateFinding)
        .with_checkpoint(CheckpointKind::Code, RerouteTarget::Implement));
    reasons.push("review");

    // Land is a fixed last block — every pipeline ends by landing/submitting the
    // work, and it's locked (can't be removed or moved).
    let _ = needs_submit; // submit intent is implied; Land always closes the pipeline
    blocks.push(StageBlock::simple("land", "Land", StageKind::Submit).locked());
    reasons.push("land");

    ProposedPipeline {
        reason: format!("Composed {} blocks from the task: {}.", blocks.len(), reasons.join(" → ")),
        blocks,
    }
}

/// Progress through a linear (non-POC) pipeline, tracked by block id rather than
/// the engine's fixed `StageId` enum. This is the execution state for arbitrary
/// templates — it lives alongside (not inside) the POC engine, so generalizing
/// execution never touches the Loopy state machine.
///
/// Minimal by design: a cursor over the template's stages plus a record of which
/// completed. Checkpoints and gates are consulted from the template's blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearRun {
    pub template_id: String,
    /// Block id currently active, or `None` before start / after completion.
    pub current: Option<String>,
    /// Block ids that have completed, in order.
    pub completed: Vec<String>,
    /// Whether the run reached the end of the pipeline.
    pub done: bool,
    /// Whether the optional stages are included in this run.
    pub include_optional: bool,
}

impl LinearRun {
    /// Begin a run of `template`, positioned at its first (non-skipped) stage.
    pub fn start(template: &PipelineTemplate, include_optional: bool) -> Self {
        let current = template
            .stages
            .iter()
            .find(|s| include_optional || !s.optional)
            .map(|s| s.id.clone());
        LinearRun {
            template_id: template.id.clone(),
            done: current.is_none(),
            current,
            completed: vec![],
            include_optional,
        }
    }

    /// Mark the current stage complete and advance to the next (respecting the
    /// opt-in setting). Returns the new current stage id, or `None` at the end.
    /// No-op if already done or not started.
    pub fn advance<'a>(&mut self, template: &'a PipelineTemplate) -> Option<&'a StageBlock> {
        let Some(cur) = self.current.clone() else {
            return None;
        };
        self.completed.push(cur.clone());
        match template.next_stage(&cur, self.include_optional) {
            Some(next) => {
                self.current = Some(next.id.clone());
                Some(next)
            }
            None => {
                self.current = None;
                self.done = true;
                None
            }
        }
    }

    /// The block the run is currently on, if any.
    pub fn current_block<'a>(&self, template: &'a PipelineTemplate) -> Option<&'a StageBlock> {
        self.current.as_ref().and_then(|id| template.stage(id))
    }
}

/// What the linear driver should do next after the current block completes.
/// Pure decision — the engine_runner performs the I/O (spawn / pause / finish).
#[derive(Debug, Clone, PartialEq)]
pub enum DriverAction {
    /// Spawn this block next (no checkpoint before it).
    Spawn(StageBlock),
    /// Pause for human review of this block before running it.
    PauseForCheckpoint(StageBlock),
    /// Pipeline finished.
    Done,
}

/// Given a run that just completed its current block, decide the next action and
/// advance the run. A block with a checkpoint pauses BEFORE it runs (so the human
/// reviews prior outputs first); otherwise it spawns. This is the pure core of
/// the driver loop, testable without any process I/O.
pub fn driver_next(run: &mut LinearRun, template: &PipelineTemplate) -> DriverAction {
    match run.advance(template) {
        Some(next) => {
            if next.checkpoint.is_some() {
                DriverAction::PauseForCheckpoint(next.clone())
            } else {
                DriverAction::Spawn(next.clone())
            }
        }
        None => DriverAction::Done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poc_template_matches_live_pipeline_order() {
        // Regression guard: the POC template must describe today's Loopy flow.
        let t = poc_from_design_doc();
        let ids: Vec<&str> = t.stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["scan", "plan", "tracks", "flight_check", "test_flight", "land"]);
    }

    #[test]
    fn poc_plan_and_flight_check_are_checkpoints() {
        let t = poc_from_design_doc();
        assert_eq!(t.stage("plan").unwrap().checkpoint, Some(CheckpointKind::Plan));
        assert_eq!(t.stage("flight_check").unwrap().checkpoint, Some(CheckpointKind::Code));
        // Flight Check rejection re-runs implementation (Loopy: back to Tracks).
        assert_eq!(t.stage("flight_check").unwrap().on_reject, Some(RerouteTarget::Implement));
    }

    #[test]
    fn test_flight_is_optional() {
        let t = poc_from_design_doc();
        assert!(t.stage("test_flight").unwrap().optional);
        // Other POC stages are mandatory.
        assert!(!t.stage("scan").unwrap().optional);
        assert!(!t.stage("land").unwrap().optional);
    }

    #[test]
    fn coding_task_has_adversarial_review_before_submit() {
        let t = coding_task();
        let ids: Vec<&str> = t.stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["implement", "beta_test", "adversarial_review", "submit_cr"]);
        // Adversarial review escalates findings and can loop back to implement.
        let rev = t.stage("adversarial_review").unwrap();
        assert_eq!(rev.gate, GatePolicy::EscalateFinding);
        assert_eq!(rev.on_reject, Some(RerouteTarget::Implement));
    }

    #[test]
    fn linear_run_walks_coding_task_to_completion() {
        let t = coding_task();
        let mut run = LinearRun::start(&t, true);
        // Starts on implement.
        assert_eq!(run.current.as_deref(), Some("implement"));
        assert_eq!(run.current_block(&t).unwrap().id, "implement");

        assert_eq!(run.advance(&t).unwrap().id, "beta_test");
        assert_eq!(run.advance(&t).unwrap().id, "adversarial_review");
        assert_eq!(run.advance(&t).unwrap().id, "submit_cr");
        // Advancing off the last stage finishes the run.
        assert!(run.advance(&t).is_none());
        assert!(run.done);
        assert_eq!(run.completed, vec!["implement", "beta_test", "adversarial_review", "submit_cr"]);
        // Further advances are no-ops.
        assert!(run.advance(&t).is_none());
    }

    #[test]
    fn driver_walks_coding_task_with_checkpoint_at_review() {
        let t = coding_task();
        let mut run = LinearRun::start(&t, true); // on implement
        // implement done → beta_test (no checkpoint) → spawn
        assert_eq!(driver_next(&mut run, &t), DriverAction::Spawn(t.stage("beta_test").unwrap().clone()));
        // beta_test done → adversarial_review HAS a checkpoint → pause
        assert_eq!(driver_next(&mut run, &t), DriverAction::PauseForCheckpoint(t.stage("adversarial_review").unwrap().clone()));
        // review done → submit_cr (no checkpoint) → spawn
        assert_eq!(driver_next(&mut run, &t), DriverAction::Spawn(t.stage("submit_cr").unwrap().clone()));
        // submit done → end.
        assert_eq!(driver_next(&mut run, &t), DriverAction::Done);
    }

    #[test]
    fn full_driver_sequence_produces_correct_spawn_order() {
        // End-to-end logic proof: drive a run to completion, recording the order
        // blocks are spawned (treating checkpoints as approved, which hook C gates).
        // This is the exact sequence the engine_runner executes via spawn_block.
        let t = coding_task();
        let mut run = LinearRun::start(&t, true);
        let mut spawned = vec![run.current.clone().unwrap()]; // first block spawns on start
        loop {
            match driver_next(&mut run, &t) {
                DriverAction::Spawn(b) | DriverAction::PauseForCheckpoint(b) => spawned.push(b.id),
                DriverAction::Done => break,
            }
        }
        assert_eq!(spawned, vec!["implement", "beta_test", "adversarial_review", "submit_cr"]);
        assert!(run.done);
    }

    #[test]
    fn linear_run_skips_optional_when_excluded() {
        let t = coding_task();
        let mut run = LinearRun::start(&t, false); // beta_test is optional
        assert_eq!(run.current.as_deref(), Some("implement"));
        assert_eq!(run.advance(&t).unwrap().id, "adversarial_review"); // beta_test skipped
        assert_eq!(run.advance(&t).unwrap().id, "submit_cr");
        assert!(run.advance(&t).is_none());
        assert!(!run.completed.contains(&"beta_test".to_string()));
    }

    #[test]
    fn poc_and_coding_task_are_executable() {
        // POC runs via the engine state machine; coding-task via the linear driver.
        assert!(poc_from_design_doc().executable);
        assert!(coding_task().executable);
    }

    #[test]
    fn from_blocks_builds_custom_executable_pipeline() {
        // A user-edited block list → a runnable custom pipeline.
        let blocks = vec![
            ("scan".into(), "scan".into(), None),
            ("design".into(), "design".into(), None),
            ("team_review".into(), "team_review".into(), Some("Team review".into())),
            ("build".into(), "implement".into(), None),
        ];
        let t = PipelineTemplate::from_blocks(&blocks);
        assert!(t.executable);
        assert_eq!(t.id, "custom");
        let ids: Vec<&str> = t.stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["scan", "design", "team_review", "build"]);
        // team_review gets a default checkpoint.
        assert!(t.stage("team_review").unwrap().checkpoint.is_some());
        // custom labels respected.
        assert_eq!(t.stage("team_review").unwrap().label, "Team review");
    }

    #[test]
    fn from_blocks_skips_unknown_kinds() {
        let blocks = vec![
            ("a".into(), "scan".into(), None),
            ("b".into(), "bogus_kind".into(), None),
            ("c".into(), "submit".into(), None),
        ];
        let t = PipelineTemplate::from_blocks(&blocks);
        let ids: Vec<&str> = t.stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]); // bogus skipped
    }

    #[test]
    fn stage_kind_from_id_roundtrip() {
        assert_eq!(StageKind::from_id("design"), Some(StageKind::Design));
        assert_eq!(StageKind::from_id("team_review"), Some(StageKind::TeamReview));
        assert_eq!(StageKind::from_id("tracks"), Some(StageKind::Implement)); // alias
        assert_eq!(StageKind::from_id("nope"), None);
    }

    #[test]
    fn stage_kind_serde_string_roundtrips_via_from_id() {
        // CRITICAL for the propose→edit→run flow: the UI receives a block's `kind`
        // as the serde-serialized string (snake_case) and sends it back on create,
        // where from_id() must parse it. If serde rename and from_id ever diverge,
        // the create flow silently drops blocks. Lock them together.
        let all = [
            StageKind::Scan, StageKind::Plan, StageKind::Design, StageKind::Implement,
            StageKind::TeamReview, StageKind::Review, StageKind::BetaTest,
            StageKind::Verify, StageKind::Submit,
        ];
        for kind in all {
            // serde serializes the enum to a JSON string (e.g. "team_review").
            let s = serde_json::to_value(kind).unwrap();
            let kind_str = s.as_str().expect("StageKind serializes to a string");
            assert_eq!(
                StageKind::from_id(kind_str), Some(kind),
                "serde string {kind_str:?} must round-trip through from_id"
            );
        }
    }

    #[test]
    fn design_task_composes_new_block_types() {
        let t = design_task();
        let ids: Vec<&str> = t.stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["scan", "plan", "design", "team_review", "tracks"]);
        // The new block types are usable in arbitrary arrangements.
        assert_eq!(t.stage("design").unwrap().kind, StageKind::Design);
        assert_eq!(t.stage("team_review").unwrap().kind, StageKind::TeamReview);
        assert!(t.stage("team_review").unwrap().checkpoint.is_some());
    }

    #[test]
    fn new_block_kinds_have_distinct_promises() {
        // Design and TeamReview must have their own completion promises so a
        // generic driver can tell them apart from other blocks.
        assert_eq!(StageKind::Design.completion_promise(), "design.complete");
        assert_eq!(StageKind::TeamReview.completion_promise(), "team_review.complete");
        assert!(!StageKind::Design.role_line().is_empty());
        assert!(!StageKind::TeamReview.role_line().is_empty());
    }

    #[test]
    fn templates_have_unique_ids() {
        let ts = builtin_templates();
        let mut ids: Vec<&str> = ts.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ts.len());
    }

    #[test]
    fn next_stage_skips_optional_when_excluded() {
        let t = poc_from_design_doc();
        // Including optional: tracks → flight_check → test_flight → land.
        assert_eq!(t.next_stage("flight_check", true).unwrap().id, "test_flight");
        // Excluding optional: flight_check → land (Test Flight skipped).
        assert_eq!(t.next_stage("flight_check", false).unwrap().id, "land");
    }

    #[test]
    fn next_stage_returns_none_at_end() {
        let t = poc_from_design_doc();
        assert!(t.next_stage("land", true).is_none());
    }

    #[test]
    fn first_stage_and_ordering() {
        let t = poc_from_design_doc();
        assert_eq!(t.first_stage().unwrap().id, "scan");
        assert!(t.is_before("scan", "land"));
        assert!(!t.is_before("land", "scan"));
        assert!(!t.is_before("scan", "nonexistent"));
    }

    #[test]
    fn template_lookup_works() {
        assert!(template_by_id("poc-from-design-doc").is_some());
        assert!(template_by_id("coding-task").is_some());
        assert!(template_by_id("nonexistent").is_none());
    }

    #[test]
    fn block_default_hat_matches_engine_for_poc_stages() {
        use crate::models::StageId;
        use crate::orchestrator::hat_collection_for_stage;
        // The implement-kind block (Loopy: Tracks/OrbitalLanes) must use the same
        // hat the engine assigns today; non-implement POC stages use none.
        let t = poc_from_design_doc();
        assert_eq!(t.stage("tracks").unwrap().kind.default_hat(), hat_collection_for_stage(StageId::OrbitalLanes));
        assert_eq!(t.stage("scan").unwrap().kind.default_hat(), hat_collection_for_stage(StageId::Scan));
        assert_eq!(t.stage("plan").unwrap().kind.default_hat(), hat_collection_for_stage(StageId::Plan));
        assert_eq!(t.stage("land").unwrap().kind.default_hat(), hat_collection_for_stage(StageId::Land));
    }

    #[test]
    fn every_stage_kind_has_a_role_line() {
        for t in builtin_templates() {
            for s in &t.stages {
                assert!(!s.kind.role_line().is_empty(), "{} has empty role line", s.id);
            }
        }
    }

    #[test]
    fn completion_promises_are_unique_and_dotted() {
        // Each kind's promise must be distinct (so a generic detector can tell
        // stages apart) and follow the "<x>.complete" convention the runner uses.
        let kinds = [StageKind::Scan, StageKind::Plan, StageKind::Implement,
            StageKind::Review, StageKind::BetaTest, StageKind::Verify, StageKind::Submit];
        let mut seen = std::collections::HashSet::new();
        for k in kinds {
            let p = k.completion_promise();
            assert!(p.ends_with(".complete"), "{p} not .complete");
            assert!(seen.insert(p), "duplicate promise {p}");
        }
    }

    #[test]
    fn planner_picks_coding_task_for_focused_work() {
        let c = plan_template_for_prompt("Fix the bug in the validate() function and submit a CR");
        assert_eq!(c.template_id, "coding-task");
    }

    #[test]
    fn planner_picks_poc_for_design_doc() {
        let c = plan_template_for_prompt("Build a POC from this design doc spanning multiple packages");
        assert_eq!(c.template_id, "poc-from-design-doc");
    }

    #[test]
    fn plan_blocks_composes_design_and_team_review_when_signaled() {
        let p = plan_blocks_for_task("write a design doc and get a team review, then implement");
        let ids: Vec<&str> = p.blocks.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"design"), "design needed: {ids:?}");
        assert!(ids.contains(&"team_review"), "team review needed: {ids:?}");
        assert!(ids.contains(&"implement"));
        // Canonical order: design before team_review before implement.
        let pos = |id: &str| ids.iter().position(|x| *x == id).unwrap();
        assert!(pos("design") < pos("team_review"));
        assert!(pos("team_review") < pos("implement"));
    }

    #[test]
    fn plan_blocks_composes_beta_test_only_when_signaled() {
        // A design task that ALSO needs beta testing gets a beta_test block —
        // the compositional behavior a fixed template couldn't provide.
        let with_beta = plan_blocks_for_task("design and implement a feature, deploy to beta and run e2e tests");
        let ids: Vec<&str> = with_beta.blocks.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"beta_test"), "beta needed: {ids:?}");

        // A plain design task does NOT get beta_test.
        let no_beta = plan_blocks_for_task("write an architecture design doc");
        let ids2: Vec<&str> = no_beta.blocks.iter().map(|b| b.id.as_str()).collect();
        assert!(!ids2.contains(&"beta_test"), "no beta expected: {ids2:?}");
    }

    #[test]
    fn compose_from_kinds_bookends_and_validates() {
        // LLM planner output → pipeline. Bookended by locked scan/land; unknown ids
        // dropped; scan/submit/land in the middle ignored.
        let ids = vec!["plan".to_string(), "bogus".to_string(), "implement".to_string(),
                       "scan".to_string(), "review".to_string()];
        let p = compose_from_kinds(&ids, "test".into());
        let got: Vec<&str> = p.blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(got, vec!["scan", "plan", "implement", "review", "land"]);
        assert!(p.blocks.first().unwrap().locked && p.blocks.last().unwrap().locked);
        // Middle review block keeps its default checkpoint.
        assert!(p.blocks.iter().find(|b| b.id == "review").unwrap().checkpoint.is_some());
    }

    #[test]
    fn plan_blocks_always_bookended_by_locked_scan_and_land() {
        // Every composed pipeline starts with a locked Scan and ends with a
        // locked Land — the fixed bookends the user asked for.
        for prompt in ["fix the small bug in validate()", "build a poc", "design a feature and ship to beta"] {
            let p = plan_blocks_for_task(prompt);
            let first = p.blocks.first().unwrap();
            let last = p.blocks.last().unwrap();
            assert_eq!(first.id, "scan", "{prompt:?} must start with scan");
            assert!(first.locked, "scan must be locked");
            assert_eq!(last.id, "land", "{prompt:?} must end with land");
            assert!(last.locked, "land must be locked");
            // Middle blocks are NOT locked (freely editable).
            for b in &p.blocks[1..p.blocks.len() - 1] {
                assert!(!b.locked, "middle block {} must be editable", b.id);
            }
        }
    }

    #[test]
    fn plan_blocks_always_includes_implement() {
        for prompt in ["do the thing", "build a poc from the design doc", "refactor the parser"] {
            let p = plan_blocks_for_task(prompt);
            let ids: Vec<&str> = p.blocks.iter().map(|b| b.id.as_str()).collect();
            assert!(ids.contains(&"implement"), "{prompt:?} → {ids:?}");
        }
    }

    #[test]
    fn planner_defaults_to_poc_when_unsure() {
        let c = plan_template_for_prompt("do the thing");
        assert_eq!(c.template_id, "poc-from-design-doc");
        // Always resolves to a real template.
        assert!(template_by_id(&c.template_id).is_some());
    }

    #[test]
    fn planner_choice_resolves_to_real_template() {
        for prompt in ["refactor the parser", "implement a new endpoint", "deliver the design doc as a poc"] {
            let c = plan_template_for_prompt(prompt);
            assert!(template_by_id(&c.template_id).is_some(), "prompt {prompt:?} → unknown template");
        }
    }

    #[test]
    fn poc_template_stays_in_sync_with_engine_stages() {
        // Regression guard against drift: every executable engine stage must map
        // to a POC template block, in the same order. The template can be MORE
        // granular than the engine (e.g. `flight_check` is a review checkpoint
        // surfaced during the OrbitalLanes/tracks stage, not its own StageId), so
        // we compare against the engine-backed blocks only.
        use crate::engine::EngineState;
        use crate::models::StageId;
        let state = EngineState::new("idea", "proj");
        let engine_block_ids: Vec<&str> = state
            .stages
            .iter()
            .filter_map(|s| poc_block_id_for_stage(s.id))
            .collect();

        // Blocks in the template that are NOT pure review-checkpoint stages
        // (flight_check has no backing StageId — it's a checkpoint on tracks).
        let template = poc_from_design_doc();
        let engine_backed_ids: std::collections::HashSet<&str> = StageId::all()
            .iter()
            .filter_map(|s| poc_block_id_for_stage(*s))
            .collect();
        let template_engine_ids: Vec<&str> = template
            .stages
            .iter()
            .map(|s| s.id.as_str())
            .filter(|id| engine_backed_ids.contains(id))
            .collect();

        assert_eq!(engine_block_ids, template_engine_ids,
            "POC template drifted from engine StageId order — update pipeline::poc_from_design_doc()");
        // And the template must additionally include the flight_check checkpoint.
        assert!(template.stage("flight_check").is_some());
    }
}
