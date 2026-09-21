//! Routine authoring, trigger control, and run inspection.
//!
//! Every state this panel renders comes from journal-backed projections, so a
//! run looks the same after a restart as it did before one.

use std::collections::BTreeMap;

use iced::Element;
use iced::widget::{button, column, row, text, text_input};
use openpodium::domain::{
    AgentId, Routine, RoutineApprovalDecision, RoutineId, RoutineInputKey, RoutineRun,
    RoutineRunId, RoutineStepId, RoutineStepRun, RoutineStepState, RoutineTriggerId, Timestamp,
    Workspace,
};

#[derive(Debug, Clone)]
pub enum Message {
    SelectRoutine(Option<RoutineId>),
    EditName(String),
    EditStepName(String),
    EditStepPrompt(String),
    EditStepAgent(String),
    EditInput {
        key: String,
        value: String,
    },
    SaveRoutine,
    AddStep,
    ToggleCanvasTemplate,
    StartRun,
    ToggleTrigger {
        trigger_id: RoutineTriggerId,
        enabled: bool,
    },
    AddScheduleTrigger,
    AddFilesystemTrigger,
    SelectRun(Option<RoutineRunId>),
    Approve {
        run_id: RoutineRunId,
        step_id: RoutineStepId,
        approved: bool,
    },
    ResolveInterruption {
        run_id: RoutineRunId,
        step_id: RoutineStepId,
    },
    CancelRun(RoutineRunId),
}

/// The routine and run the user is looking at, plus the draft they are editing.
#[derive(Default)]
pub struct UiState {
    pub selected_routine: Option<RoutineId>,
    pub selected_run: Option<RoutineRunId>,
    pub name: String,
    pub step_name: String,
    pub step_prompt: String,
    pub step_agent: String,
    /// Draft values for the selected routine's declared inputs.
    pub inputs: BTreeMap<String, String>,
    /// Steps accumulated for a routine that has not been saved yet.
    pub pending_steps: Vec<PendingStep>,
    /// Whether saving should capture the current canvas selection, so every
    /// run rebuilds that arrangement with its own entity identifiers.
    pub capture_canvas: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStep {
    pub name: String,
    pub prompt: String,
    pub agent_id: AgentId,
}

impl UiState {
    pub fn select(&mut self, workspace: &Workspace, routine_id: Option<RoutineId>) {
        self.selected_routine = routine_id;
        self.selected_run = None;
        self.pending_steps.clear();
        self.inputs.clear();
        self.capture_canvas = false;
        match routine_id.and_then(|id| workspace.routine(id)) {
            Some(routine) => {
                self.name = routine.name().as_str().to_owned();
                for input in routine.latest_version().inputs() {
                    let value = input
                        .default()
                        .map(|value| value.as_str().to_owned())
                        .unwrap_or_default();
                    self.inputs.insert(input.key().as_str().to_owned(), value);
                }
            }
            None => self.name.clear(),
        }
    }

    pub fn input_values(&self) -> BTreeMap<RoutineInputKey, openpodium::domain::RoutineValue> {
        self.inputs
            .iter()
            .filter_map(|(key, value)| {
                Some((
                    RoutineInputKey::new(key.clone()).ok()?,
                    openpodium::domain::RoutineValue::new(value.clone()).ok()?,
                ))
            })
            .collect()
    }
}

pub fn panel<'a>(workspace: &'a Workspace, state: &'a UiState) -> Element<'a, Message> {
    let mut content = column![text("Routines").size(24)].spacing(8);

    let mut selector = row![
        button(if state.selected_routine.is_none() {
            "✓ New routine"
        } else {
            "New routine"
        })
        .on_press(Message::SelectRoutine(None)),
    ]
    .spacing(6);
    for routine in workspace.routines() {
        let label = format!(
            "{} · v{}",
            routine.name(),
            routine.latest_version().number()
        );
        selector = selector.push(
            button(text(if state.selected_routine == Some(routine.id()) {
                format!("✓ {label}")
            } else {
                label
            }))
            .on_press(Message::SelectRoutine(Some(routine.id()))),
        );
    }
    content = content.push(selector.wrap());

    content = content.push(editor(workspace, state));

    if let Some(routine) = state.selected_routine.and_then(|id| workspace.routine(id)) {
        content = content.push(inputs_section(routine, state));
        content = content.push(triggers_section(routine));
    }

    content = content.push(runs_section(workspace, state));
    content.into()
}

/// Editing a saved routine appends a version; it never rewrites the one that
/// running work pinned, and the panel says so.
fn editor<'a>(workspace: &'a Workspace, state: &'a UiState) -> Element<'a, Message> {
    let saving_existing = state.selected_routine.is_some();
    let mut editor = column![
        text(if saving_existing {
            "Edit routine (saves a new version)"
        } else {
            "New routine"
        })
        .size(18),
        text_input("Routine name", &state.name).on_input(Message::EditName),
        row![
            text_input("Step name", &state.step_name).on_input(Message::EditStepName),
            text_input("Agent ID", &state.step_agent).on_input(Message::EditStepAgent),
        ]
        .spacing(6),
        text_input(
            "Step prompt; use {{name}} for a bound input",
            &state.step_prompt
        )
        .on_input(Message::EditStepPrompt),
        row![
            button("Add step").on_press(Message::AddStep),
            button(if state.capture_canvas {
                "✓ Rebuild canvas selection per run"
            } else {
                "Rebuild canvas selection per run"
            })
            .on_press(Message::ToggleCanvasTemplate),
            button(if saving_existing {
                "Save new version"
            } else {
                "Save routine"
            })
            .on_press(Message::SaveRoutine),
        ]
        .spacing(6),
    ]
    .spacing(6);

    if !state.pending_steps.is_empty() {
        editor = editor.push(text("Unsaved steps").size(14));
        for (index, step) in state.pending_steps.iter().enumerate() {
            editor = editor.push(text(format!(
                "{}. {} → agent {} · {}",
                index + 1,
                step.name,
                step.agent_id,
                step.prompt
            )));
        }
        editor = editor.push(
            text("Steps run in the order listed; each depends on the one before it.").size(12),
        );
    }

    if let Some(routine) = state.selected_routine.and_then(|id| workspace.routine(id)) {
        editor = editor.push(text("Saved steps").size(14));
        for step in routine.latest_version().steps() {
            let approval = if step.approval() == openpodium::domain::RoutineApproval::Required {
                " · approval required"
            } else {
                ""
            };
            editor = editor.push(text(format!(
                "{} → agent {}{approval}",
                step.name(),
                step.agent_id()
            )));
        }
    }
    editor.into()
}

fn inputs_section<'a>(routine: &'a Routine, state: &'a UiState) -> Element<'a, Message> {
    let declarations = routine.latest_version().inputs();
    if declarations.is_empty() {
        return column![].into();
    }
    let mut section = column![text("Inputs").size(18)].spacing(6);
    for declaration in declarations {
        let key = declaration.key().as_str().to_owned();
        let current = state.inputs.get(&key).cloned().unwrap_or_default();
        let label = if declaration.required() {
            format!("{} (required)", declaration.label())
        } else {
            declaration.label().as_str().to_owned()
        };
        section = section.push(
            row![
                text(label),
                text_input("value", &current).on_input(move |value| Message::EditInput {
                    key: key.clone(),
                    value,
                }),
            ]
            .spacing(6),
        );
    }
    section.into()
}

fn triggers_section(routine: &Routine) -> Element<'_, Message> {
    let mut section = column![
        text("Triggers").size(18),
        text("Triggers only fire while OpenPodium is open. Occurrences that pass while it is closed are skipped and counted.")
            .size(12),
    ]
    .spacing(6);
    for trigger in routine.triggers() {
        let mut detail = format!("{} · {}", trigger.name(), trigger.kind().label());
        if let Some(schedule) = trigger.schedule() {
            let zone = schedule
                .timezone_label()
                .map(|label| label.as_str().to_owned())
                .unwrap_or_else(|| format_timezone_offset(schedule.offset_minutes()));
            detail.push_str(&format!(
                " · {zone} · next {}",
                schedule
                    .next_occurrence()
                    .map_or_else(|| "unscheduled".to_owned(), format_instant)
            ));
        }
        if trigger.skipped_occurrences() > 0 {
            detail.push_str(&format!(
                " · {} missed while closed",
                trigger.skipped_occurrences()
            ));
        }
        let trigger_id = trigger.id();
        section = section.push(
            row![
                button(if trigger.enabled() {
                    "Disable"
                } else {
                    "Enable"
                })
                .on_press(Message::ToggleTrigger {
                    trigger_id,
                    enabled: !trigger.enabled(),
                }),
                text(detail),
            ]
            .spacing(6),
        );
    }
    section = section.push(
        row![
            button("Run now").on_press(Message::StartRun),
            button("Add hourly schedule").on_press(Message::AddScheduleTrigger),
            button("Watch files").on_press(Message::AddFilesystemTrigger),
        ]
        .spacing(6),
    );
    section.into()
}

fn format_timezone_offset(offset_minutes: i32) -> String {
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let absolute = offset_minutes.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", absolute / 60, absolute % 60)
}

fn runs_section<'a>(workspace: &'a Workspace, state: &'a UiState) -> Element<'a, Message> {
    let mut section = column![text("Runs").size(18)].spacing(6);
    let runs: Vec<&RoutineRun> = workspace
        .routine_runs()
        .filter(|run| {
            state
                .selected_routine
                .is_none_or(|routine_id| run.routine_id() == routine_id)
        })
        .collect();
    if runs.is_empty() {
        return section.push(text("No runs yet.")).into();
    }

    for run in runs {
        let label = format!(
            "Run {} · v{} · {}",
            run.id(),
            run.pin().version().number(),
            run.state()
        );
        let mut header = row![
            button(text(if state.selected_run == Some(run.id()) {
                format!("✓ {label}")
            } else {
                label
            }))
            .on_press(Message::SelectRun(Some(run.id()))),
        ]
        .spacing(6);
        if run.is_active() {
            header = header.push(button("Cancel run").on_press(Message::CancelRun(run.id())));
        }
        section = section.push(header.wrap());

        if state.selected_run != Some(run.id()) {
            continue;
        }
        for step in run.steps() {
            section = section.push(step_row(workspace, run, step));
        }
        if !run.pin().revisions().is_empty() {
            section = section.push(text("Pinned revisions").size(14));
            for (checkout, revision) in run.pin().revisions() {
                let short: String = revision.chars().take(12).collect();
                section = section.push(text(format!("{checkout} @ {short}")).size(12));
            }
        }
    }
    section.into()
}

fn step_row<'a>(
    workspace: &'a Workspace,
    run: &'a RoutineRun,
    step: &'a RoutineStepRun,
) -> Element<'a, Message> {
    let definition = run.pin().version().step(step.step_id());
    let title = definition.map_or_else(
        || format!("Step {}", step.step_id()),
        |step| step.name().as_str().to_owned(),
    );
    let attempts = step.attempts().len();
    let mut detail = format!("{title} · {}", step.state());
    if attempts > 1 {
        detail.push_str(&format!(" · attempt {attempts}"));
    }
    if let Some(task_id) = step.attempts().last().map(|attempt| attempt.task_id())
        && let Some(task) = workspace.task(task_id)
    {
        detail.push_str(&format!(" · task {task_id} is {}", task.state()));
    }
    if let Some(interruption) = step.interruption() {
        detail.push_str(&format!(" · {}", interruption.reason()));
    }
    if let Some(failure) = step.failure() {
        detail.push_str(&format!(" · {}", failure.as_str()));
    }

    let mut controls = row![text(detail)].spacing(6);
    match step.state() {
        RoutineStepState::AwaitingApproval => {
            controls = controls
                .push(button("Approve").on_press(Message::Approve {
                    run_id: run.id(),
                    step_id: step.step_id(),
                    approved: true,
                }))
                .push(button("Reject").on_press(Message::Approve {
                    run_id: run.id(),
                    step_id: step.step_id(),
                    approved: false,
                }));
        }
        RoutineStepState::Interrupted => {
            controls = controls.push(button("Confirm stopped").on_press(
                Message::ResolveInterruption {
                    run_id: run.id(),
                    step_id: step.step_id(),
                },
            ));
        }
        _ => {}
    }
    let mut block = column![controls.wrap()].spacing(2);
    if !step.outputs().is_empty() {
        let outputs = step
            .outputs()
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        block = block.push(text(format!("outputs: {outputs}")).size(12));
    }
    block.into()
}

/// Renders an instant as a UTC calendar time.
///
/// OpenPodium carries no date library and no timezone database, so this shows
/// UTC and the schedule prints the offset it stored beside it. The conversion
/// is Howard Hinnant's civil-from-days algorithm.
fn format_instant(instant: Timestamp) -> String {
    let seconds = instant.as_unix_millis() / 1_000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let time = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        time / 3_600,
        (time % 3_600) / 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * shifted_month + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    })
    .unwrap_or(1);
    (year + i64::from(month <= 2), month, day)
}

/// Converts a checkbox decision into the domain's approval decision.
pub const fn decision(approved: bool) -> RoutineApprovalDecision {
    if approved {
        RoutineApprovalDecision::Approved
    } else {
        RoutineApprovalDecision::Rejected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_render_as_utc_calendar_times() {
        assert_eq!(
            format_instant(Timestamp::from_unix_millis(0)),
            "1970-01-01 00:00 UTC"
        );
        // 2026-09-20T14:30:00Z
        assert_eq!(
            format_instant(Timestamp::from_unix_millis(1_789_914_600_000)),
            "2026-09-20 14:30 UTC"
        );
        // A leap day, which the era arithmetic has to get right.
        assert_eq!(
            format_instant(Timestamp::from_unix_millis(1_709_164_800_000)),
            "2024-02-29 00:00 UTC"
        );
    }

    #[test]
    fn timezone_offsets_keep_hour_and_minute_components() {
        assert_eq!(format_timezone_offset(330), "UTC+05:30");
        assert_eq!(format_timezone_offset(-30), "UTC-00:30");
        assert_eq!(format_timezone_offset(0), "UTC+00:00");
    }
}
