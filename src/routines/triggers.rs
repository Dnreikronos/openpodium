//! Local trigger adapters.
//!
//! Every trigger is observed by this process. OpenPodium does not run in the
//! background, so nothing fires while it is closed; a schedule that came due
//! then is skipped and counted rather than replayed in a burst at startup.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::UNIX_EPOCH;

use crate::domain::{
    Routine, RoutineCadence, RoutineId, RoutineInputKey, RoutineOccurrenceKey, RoutineTrigger,
    RoutineTriggerId, RoutineTriggerKind, RoutineValue, Timestamp, Workspace, WorkspaceId,
};
use crate::workspaces::WorkspaceManager;

use super::TriggerFiring;

/// Guards against a filesystem trigger walking an unbounded tree.
const MAX_SCANNED_ENTRIES: usize = 20_000;
/// Guards against an ancient `next_occurrence` producing an unbounded loop.
const MAX_CATCHUP_OCCURRENCES: u32 = 10_000;

/// One thing the watcher observed and thinks should start a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerEvent {
    pub workspace_id: WorkspaceId,
    pub routine_id: RoutineId,
    pub firing: TriggerFiring,
    /// Context the trigger observed, offered to the routine's declared inputs.
    pub observed: BTreeMap<RoutineInputKey, RoutineValue>,
}

/// Schedule occurrences that passed while OpenPodium was closed. They are
/// recorded, not run, so the limitation stays visible instead of silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissedOccurrences {
    pub workspace_id: WorkspaceId,
    pub routine_id: RoutineId,
    pub trigger_id: RoutineTriggerId,
    pub skipped: u32,
    pub next_occurrence: Option<Timestamp>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TriggerPoll {
    pub events: Vec<TriggerEvent>,
    pub missed: Vec<MissedOccurrences>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilesystemObservation {
    digest: String,
    /// When the current digest was first seen. A change only fires once it has
    /// been stable for the trigger's debounce window, so a burst of edits
    /// produces one run instead of one per keystroke.
    first_seen: Timestamp,
    fired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitObservation {
    /// The revisions the last consumed run was compared against. It only moves
    /// once a run actually starts, so a transient failure re-reports the change
    /// instead of losing it.
    baseline: BTreeMap<String, String>,
    pending: Option<BTreeMap<String, String>>,
}

#[derive(Default)]
pub struct TriggerWatcher {
    /// When this session began. A schedule occurrence earlier than this passed
    /// while OpenPodium was closed.
    session_start: Option<Timestamp>,
    filesystem: BTreeMap<(u64, u64), FilesystemObservation>,
    git: BTreeMap<(u64, u64), GitObservation>,
}

impl TriggerWatcher {
    pub fn new(session_start: Timestamp) -> Self {
        Self {
            session_start: Some(session_start),
            filesystem: BTreeMap::new(),
            git: BTreeMap::new(),
        }
    }

    /// Observes every enabled trigger once.
    ///
    /// This only reports what it saw. Consuming an occurrence and creating its
    /// run happen together in one journal transaction, in the scheduler.
    pub fn poll(&mut self, workspaces: &WorkspaceManager, now: Timestamp) -> TriggerPoll {
        let session_start = *self.session_start.get_or_insert(now);
        let mut poll = TriggerPoll::default();
        let observations: Vec<_> = workspaces
            .recent_workspaces()
            .flat_map(|workspace| {
                workspace.routines().flat_map(move |routine| {
                    routine
                        .triggers()
                        .filter(|trigger| trigger.enabled())
                        .map(move |trigger| (workspace, routine, trigger))
                })
            })
            .map(|(workspace, routine, trigger)| {
                (workspace.id(), routine.id(), trigger.clone(), workspace)
            })
            .collect();

        for (workspace_id, routine_id, trigger, workspace) in observations {
            match trigger.kind() {
                RoutineTriggerKind::Manual => {}
                RoutineTriggerKind::Filesystem {
                    patterns,
                    debounce_ms,
                } => {
                    if let Some(event) = self.poll_filesystem(
                        workspace,
                        workspace_id,
                        routine_id,
                        &trigger,
                        patterns,
                        *debounce_ms,
                        now,
                    ) {
                        poll.events.push(event);
                    }
                }
                RoutineTriggerKind::Git { refs } => {
                    if let Some(event) =
                        self.poll_git(workspace, workspace_id, routine_id, &trigger, refs)
                    {
                        poll.events.push(event);
                    }
                }
                RoutineTriggerKind::Schedule(schedule) => {
                    let outcome = poll_schedule(schedule, trigger.id(), session_start, now);
                    if outcome.skipped > 0 {
                        poll.missed.push(MissedOccurrences {
                            workspace_id,
                            routine_id,
                            trigger_id: trigger.id(),
                            skipped: outcome.skipped,
                            next_occurrence: outcome.next_occurrence,
                        });
                    }
                    if let Some(occurrence) = outcome.firing {
                        poll.events.push(TriggerEvent {
                            workspace_id,
                            routine_id,
                            firing: TriggerFiring {
                                trigger_id: trigger.id(),
                                occurrence,
                                next_occurrence: outcome.next_occurrence,
                            },
                            observed: BTreeMap::new(),
                        });
                    }
                }
            }
        }
        poll
    }

    /// Reports that an event was consumed, so the same observation does not
    /// produce a second run.
    pub fn mark_consumed(&mut self, event: &TriggerEvent) {
        let key = (event.workspace_id.get(), event.firing.trigger_id.get());
        if let Some(observation) = self.filesystem.get_mut(&key) {
            observation.fired = true;
        }
        if let Some(observation) = self.git.get_mut(&key)
            && let Some(pending) = observation.pending.take()
        {
            observation.baseline = pending;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn poll_filesystem(
        &mut self,
        workspace: &Workspace,
        workspace_id: WorkspaceId,
        routine_id: RoutineId,
        trigger: &RoutineTrigger,
        patterns: &[String],
        debounce_ms: u64,
        now: Timestamp,
    ) -> Option<TriggerEvent> {
        let root = workspace.active_directory()?;
        let digest = scan_digest(Path::new(root.as_str()), patterns)?;
        let key = (workspace_id.get(), trigger.id().get());
        let observation = self
            .filesystem
            .entry(key)
            .or_insert_with(|| FilesystemObservation {
                digest: digest.clone(),
                first_seen: now,
                // The state at the first observation is the baseline, not a
                // change: opening a workspace must not start every routine.
                fired: true,
            });
        if observation.digest != digest {
            *observation = FilesystemObservation {
                digest: digest.clone(),
                first_seen: now,
                fired: false,
            };
        }
        if observation.fired {
            return None;
        }
        if now
            .as_unix_millis()
            .saturating_sub(observation.first_seen.as_unix_millis())
            < debounce_ms
        {
            return None;
        }
        // The occurrence identity is the observed state, so a duplicate
        // observation of the same tree can never start a second run.
        let occurrence =
            RoutineOccurrenceKey::new(format!("fs-{}-{digest}", trigger.id().get())).ok()?;
        Some(TriggerEvent {
            workspace_id,
            routine_id,
            firing: TriggerFiring {
                trigger_id: trigger.id(),
                occurrence,
                next_occurrence: None,
            },
            observed: BTreeMap::new(),
        })
    }

    fn poll_git(
        &mut self,
        workspace: &Workspace,
        workspace_id: WorkspaceId,
        routine_id: RoutineId,
        trigger: &RoutineTrigger,
        refs: &[String],
    ) -> Option<TriggerEvent> {
        let root = workspace.active_directory()?;
        let observed = read_refs(Path::new(root.as_str()), refs);
        let key = (workspace_id.get(), trigger.id().get());
        // The first observation is the baseline. Without it, opening a
        // workspace would look like every watched ref had just changed.
        let Some(observation) = self.git.get_mut(&key) else {
            self.git.insert(
                key,
                GitObservation {
                    baseline: observed,
                    pending: None,
                },
            );
            return None;
        };
        observation.pending = Some(observed.clone());

        let (reference, after) = observed.iter().find(|(reference, revision)| {
            observation.baseline.get(*reference) != Some(*revision)
        })?;
        let before = observation
            .baseline
            .get(reference)
            .cloned()
            .unwrap_or_else(|| "none".to_owned());
        let occurrence = RoutineOccurrenceKey::new(format!(
            "git-{}-{}",
            trigger.id().get(),
            short_identity(reference, after)
        ))
        .ok()?;
        let mut context = BTreeMap::new();
        for (key, value) in [
            ("git.ref", reference.clone()),
            ("git.before", before),
            ("git.after", after.clone()),
        ] {
            if let (Ok(key), Ok(value)) = (RoutineInputKey::new(key), RoutineValue::new(value)) {
                context.insert(key, value);
            }
        }
        Some(TriggerEvent {
            workspace_id,
            routine_id,
            firing: TriggerFiring {
                trigger_id: trigger.id(),
                occurrence,
                next_occurrence: None,
            },
            observed: context,
        })
    }
}

struct ScheduleOutcome {
    firing: Option<RoutineOccurrenceKey>,
    skipped: u32,
    next_occurrence: Option<Timestamp>,
}

/// Decides what a schedule owes at `now`.
///
/// Occurrences that fell before this session started passed while OpenPodium
/// was closed and are counted as skipped. At most one occurrence fires per
/// poll; repeated triggers coalesce onto the run already in flight.
fn poll_schedule(
    schedule: &crate::domain::RoutineSchedule,
    trigger_id: RoutineTriggerId,
    session_start: Timestamp,
    now: Timestamp,
) -> ScheduleOutcome {
    let cadence = schedule.cadence();
    let offset = schedule.offset_minutes();
    let Some(mut candidate) = schedule
        .next_occurrence()
        .or_else(|| cadence.next_occurrence(now, offset))
    else {
        return ScheduleOutcome {
            firing: None,
            skipped: 0,
            next_occurrence: None,
        };
    };
    if candidate > now {
        return ScheduleOutcome {
            firing: None,
            skipped: 0,
            next_occurrence: Some(candidate),
        };
    }

    let mut skipped = 0;
    let mut firing = None;
    let mut guard = 0;
    while candidate <= now && guard < MAX_CATCHUP_OCCURRENCES {
        guard += 1;
        if candidate >= session_start && firing.is_none() {
            firing = RoutineOccurrenceKey::new(format!(
                "sched-{}-{}",
                trigger_id.get(),
                candidate.as_unix_millis()
            ))
            .ok();
        } else if firing.is_none() {
            skipped += 1;
        } else {
            // A later due occurrence while one is already firing is coalesced
            // into the run that is about to start.
            skipped += 1;
        }
        let Some(next) = advance_occurrence(cadence, candidate, offset) else {
            break;
        };
        candidate = next;
    }
    ScheduleOutcome {
        firing,
        skipped,
        next_occurrence: Some(candidate),
    }
}

fn advance_occurrence(cadence: RoutineCadence, from: Timestamp, offset: i32) -> Option<Timestamp> {
    let next = from.as_unix_millis().checked_add(1)?;
    cadence.next_occurrence(Timestamp::from_unix_millis(next), offset)
}

/// Hashes the paths a filesystem trigger watches together with their size and
/// modification time. Two observations of the same tree produce the same value.
fn scan_digest(root: &Path, patterns: &[String]) -> Option<String> {
    let mut hasher = blake3::Hasher::new();
    let mut stack = vec![root.to_path_buf()];
    let mut scanned = 0_usize;
    let mut matched = BTreeSet::new();
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            scanned += 1;
            if scanned > MAX_SCANNED_ENTRIES {
                break;
            }
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                // Git's own directory churns constantly and is never source.
                if relative.split('/').next_back() != Some(".git") {
                    stack.push(path);
                }
                continue;
            }
            if !patterns
                .iter()
                .any(|pattern| matches_pattern(pattern, &relative))
            {
                continue;
            }
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_millis());
            matched.insert(format!("{relative}:{}:{modified}", metadata.len()));
        }
    }
    for entry in &matched {
        hasher.update(entry.as_bytes());
        hasher.update(b"\n");
    }
    Some(hasher.finalize().to_hex()[..32].to_owned())
}

/// A small glob matcher: `*` matches within one path segment and `**` matches
/// any number of segments.
fn matches_pattern(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    matches_segments(&pattern, &path)
}

fn matches_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            if rest.is_empty() {
                return true;
            }
            (0..=path.len()).any(|skip| matches_segments(rest, &path[skip..]))
        }
        Some((segment, rest)) => match path.split_first() {
            Some((head, tail)) if matches_segment(segment, head) => matches_segments(rest, tail),
            _ => false,
        },
    }
}

fn matches_segment(pattern: &str, value: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return pattern == value;
    };
    if !value.starts_with(first) {
        return false;
    }
    let mut rest = &value[first.len()..];
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// Reads the current revision of each watched ref. A ref that does not exist is
/// simply absent, which is how a newly created branch shows up as a change.
fn read_refs(root: &Path, refs: &[String]) -> BTreeMap<String, String> {
    let mut revisions = BTreeMap::new();
    for reference in refs {
        let Ok(output) = Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("{reference}^{{commit}}"))
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        if let Ok(revision) = String::from_utf8(output.stdout) {
            let revision = revision.trim();
            if !revision.is_empty() {
                revisions.insert(reference.clone(), revision.to_owned());
            }
        }
    }
    revisions
}

/// A stable, identifier-safe token for a ref and the revision observed on it.
fn short_identity(reference: &str, revision: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(reference.as_bytes());
    hasher.update(b"@");
    hasher.update(revision.as_bytes());
    hasher.finalize().to_hex()[..24].to_owned()
}

/// Whether the routine still has an active run, so a repeated trigger should be
/// coalesced instead of rejected as an error.
pub fn has_active_run(workspace: &Workspace, routine_id: RoutineId) -> bool {
    workspace
        .active_routine_runs()
        .any(|run| run.routine_id() == routine_id)
}

/// The routine a trigger belongs to, for callers holding only a trigger ID.
pub fn routine_of_trigger(workspace: &Workspace, trigger_id: RoutineTriggerId) -> Option<&Routine> {
    workspace
        .routines()
        .find(|routine| routine.trigger(trigger_id).is_some())
}
