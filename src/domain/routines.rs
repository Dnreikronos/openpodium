//! Immutable routine versions and the durable state of their runs.
//!
//! A routine is a named workflow. Editing it never mutates a stored version;
//! it appends a new one. Starting a run copies the version, the resolved
//! inputs, the agent bindings, the checkout identities, and the Git revisions
//! observed at that moment, so the orchestration decisions a run makes are
//! reproducible from the journal even after the routine is edited again.
//!
//! Agent responses still vary. Determinism here means the scheduler's choices
//! and the inputs it recorded, not the text an agent produced.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::{
    AgentId, Content, HandoffId, Name, RoutineAttemptId, RoutineId, RoutineRunId, RoutineStepId,
    RoutineTriggerId, RoutineVersionId, TaskId, Timestamp, WorkspaceDirectory,
};

/// The largest DAG a single routine version may describe.
pub const MAX_ROUTINE_STEPS: usize = 128;
/// The largest number of direct dependencies one step may declare.
pub const MAX_STEP_DEPENDENCIES: usize = 32;
/// The largest number of declared inputs, step bindings, or step outputs.
pub const MAX_ROUTINE_KEYS: usize = 64;
/// The largest number of named resources one step may claim.
pub const MAX_STEP_RESOURCES: usize = 16;
/// The largest number of watched paths or refs one trigger may declare.
pub const MAX_TRIGGER_PATTERNS: usize = 64;

macro_rules! define_key {
    ($name:ident, $field:literal, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub const MAX_CHARS: usize = $max;

            pub fn new(value: impl Into<String>) -> Result<Self, RoutineError> {
                let value = value.into();
                if value.is_empty() || value.chars().count() > Self::MAX_CHARS {
                    return Err(RoutineError::InvalidKey { field: $field });
                }
                if !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
                {
                    return Err(RoutineError::InvalidKey { field: $field });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

define_key!(RoutineInputKey, "routine input key", 64);
define_key!(RoutineOutputKey, "routine output key", 64);
define_key!(RoutineResourceKey, "routine resource key", 64);
define_key!(RoutineOccurrenceKey, "routine trigger occurrence", 160);

/// A resolved input or output value. Unlike [`Content`] an empty value is
/// meaningful, because a step may legitimately produce an empty result.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoutineValue(String);

impl RoutineValue {
    pub const MAX_CHARS: usize = 16_384;

    pub fn new(value: impl Into<String>) -> Result<Self, RoutineError> {
        let value = value.into();
        if value.chars().count() > Self::MAX_CHARS {
            return Err(RoutineError::ValueTooLong {
                max_chars: Self::MAX_CHARS,
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for RoutineValue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One input the routine asks for when a run starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineInputDeclaration {
    key: RoutineInputKey,
    label: Name,
    required: bool,
    default: Option<RoutineValue>,
}

impl RoutineInputDeclaration {
    pub const fn new(
        key: RoutineInputKey,
        label: Name,
        required: bool,
        default: Option<RoutineValue>,
    ) -> Self {
        Self {
            key,
            label,
            required,
            default,
        }
    }

    pub const fn key(&self) -> &RoutineInputKey {
        &self.key
    }

    pub const fn label(&self) -> &Name {
        &self.label
    }

    pub const fn required(&self) -> bool {
        self.required
    }

    pub const fn default(&self) -> Option<&RoutineValue> {
        self.default.as_ref()
    }
}

/// Where a step binding takes its value from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineBindingSource {
    /// A run input supplied by the trigger or the user.
    Input(RoutineInputKey),
    /// A declared output of an earlier step.
    StepOutput {
        step_id: RoutineStepId,
        key: RoutineOutputKey,
    },
    /// A constant recorded in the routine version.
    Literal(RoutineValue),
}

/// Whether a step pauses for a human decision before it is dispatched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RoutineApproval {
    #[default]
    NotRequired,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutineRetryPolicy {
    max_attempts: u32,
}

impl RoutineRetryPolicy {
    pub const DEFAULT_MAX_ATTEMPTS: u32 = 1;

    pub const fn new(max_attempts: u32) -> Result<Self, RoutineError> {
        if max_attempts == 0 {
            return Err(RoutineError::InvalidRetryPolicy);
        }
        Ok(Self { max_attempts })
    }

    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }
}

impl Default for RoutineRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
        }
    }
}

/// Which checkout a step needs. The claim is declarative; a run resolves it to
/// a canonical checkout identity when it is pinned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RoutineCheckoutClaim {
    /// The checkout the bound agent's canvas node already belongs to.
    #[default]
    AgentDefault,
    /// A specific floor. Runs resolve it to that floor's directory.
    Floor(u64),
}

/// Everything a step must hold exclusively while it executes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutineStepClaims {
    checkout: RoutineCheckoutClaim,
    resources: BTreeSet<RoutineResourceKey>,
}

impl RoutineStepClaims {
    pub fn new(
        checkout: RoutineCheckoutClaim,
        resources: impl IntoIterator<Item = RoutineResourceKey>,
    ) -> Result<Self, RoutineError> {
        let resources: BTreeSet<_> = resources.into_iter().collect();
        if resources.len() > MAX_STEP_RESOURCES {
            return Err(RoutineError::TooMany {
                field: "step resource claims",
                max: MAX_STEP_RESOURCES,
            });
        }
        Ok(Self {
            checkout,
            resources,
        })
    }

    pub const fn checkout(&self) -> RoutineCheckoutClaim {
        self.checkout
    }

    pub fn resources(&self) -> impl Iterator<Item = &RoutineResourceKey> {
        self.resources.iter()
    }
}

/// One node of the routine DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineStep {
    id: RoutineStepId,
    name: Name,
    agent_id: AgentId,
    prompt: Content,
    depends_on: BTreeSet<RoutineStepId>,
    bindings: BTreeMap<RoutineInputKey, RoutineBindingSource>,
    outputs: BTreeSet<RoutineOutputKey>,
    approval: RoutineApproval,
    retry: RoutineRetryPolicy,
    claims: RoutineStepClaims,
}

impl RoutineStep {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: RoutineStepId,
        name: Name,
        agent_id: AgentId,
        prompt: Content,
        depends_on: impl IntoIterator<Item = RoutineStepId>,
        bindings: impl IntoIterator<Item = (RoutineInputKey, RoutineBindingSource)>,
        outputs: impl IntoIterator<Item = RoutineOutputKey>,
        approval: RoutineApproval,
        retry: RoutineRetryPolicy,
        claims: RoutineStepClaims,
    ) -> Result<Self, RoutineError> {
        let depends_on: BTreeSet<_> = depends_on.into_iter().collect();
        if depends_on.len() > MAX_STEP_DEPENDENCIES {
            return Err(RoutineError::TooMany {
                field: "step dependencies",
                max: MAX_STEP_DEPENDENCIES,
            });
        }
        if depends_on.contains(&id) {
            return Err(RoutineError::DependencyCycle { step_id: id });
        }
        let bindings: BTreeMap<_, _> = bindings.into_iter().collect();
        if bindings.len() > MAX_ROUTINE_KEYS {
            return Err(RoutineError::TooMany {
                field: "step bindings",
                max: MAX_ROUTINE_KEYS,
            });
        }
        let outputs: BTreeSet<_> = outputs.into_iter().collect();
        if outputs.len() > MAX_ROUTINE_KEYS {
            return Err(RoutineError::TooMany {
                field: "step outputs",
                max: MAX_ROUTINE_KEYS,
            });
        }
        Ok(Self {
            id,
            name,
            agent_id,
            prompt,
            depends_on,
            bindings,
            outputs,
            approval,
            retry,
            claims,
        })
    }

    pub const fn id(&self) -> RoutineStepId {
        self.id
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn prompt(&self) -> &Content {
        &self.prompt
    }

    pub fn depends_on(&self) -> impl Iterator<Item = RoutineStepId> + '_ {
        self.depends_on.iter().copied()
    }

    pub fn bindings(&self) -> impl Iterator<Item = (&RoutineInputKey, &RoutineBindingSource)> {
        self.bindings.iter()
    }

    pub fn outputs(&self) -> impl Iterator<Item = &RoutineOutputKey> {
        self.outputs.iter()
    }

    pub fn declares_output(&self, key: &RoutineOutputKey) -> bool {
        self.outputs.contains(key)
    }

    pub const fn approval(&self) -> RoutineApproval {
        self.approval
    }

    pub const fn retry(&self) -> RoutineRetryPolicy {
        self.retry
    }

    pub const fn claims(&self) -> &RoutineStepClaims {
        &self.claims
    }

    /// Substitutes `{{binding}}` placeholders with resolved values. An unknown
    /// placeholder is an error so a run never dispatches a half-filled prompt.
    pub fn render_prompt(
        &self,
        values: &BTreeMap<RoutineInputKey, RoutineValue>,
    ) -> Result<Content, RoutineError> {
        let source = self.prompt.as_str();
        let mut rendered = String::with_capacity(source.len());
        let mut rest = source;
        while let Some(start) = rest.find("{{") {
            rendered.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                return Err(RoutineError::UnterminatedPlaceholder { step_id: self.id });
            };
            let name = after[..end].trim();
            let key = RoutineInputKey::new(name)?;
            let value = values
                .get(&key)
                .ok_or_else(|| RoutineError::UnboundPlaceholder {
                    step_id: self.id,
                    key: key.clone(),
                })?;
            rendered.push_str(value.as_str());
            rest = &after[end + 2..];
        }
        rendered.push_str(rest);
        Content::new(rendered).map_err(|_| RoutineError::EmptyRenderedPrompt { step_id: self.id })
    }
}

/// An immutable routine definition. Editing a routine appends a new version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineVersion {
    id: RoutineVersionId,
    routine_id: RoutineId,
    number: u32,
    inputs: Vec<RoutineInputDeclaration>,
    steps: Vec<RoutineStep>,
    template: Option<Content>,
    created_at: Timestamp,
}

impl RoutineVersion {
    pub fn new(
        id: RoutineVersionId,
        routine_id: RoutineId,
        number: u32,
        inputs: Vec<RoutineInputDeclaration>,
        mut steps: Vec<RoutineStep>,
        template: Option<Content>,
        created_at: Timestamp,
    ) -> Result<Self, RoutineError> {
        if number == 0 {
            return Err(RoutineError::InvalidVersionNumber { number });
        }
        if steps.is_empty() {
            return Err(RoutineError::EmptyRoutine { routine_id });
        }
        if steps.len() > MAX_ROUTINE_STEPS {
            return Err(RoutineError::TooMany {
                field: "routine steps",
                max: MAX_ROUTINE_STEPS,
            });
        }
        if inputs.len() > MAX_ROUTINE_KEYS {
            return Err(RoutineError::TooMany {
                field: "routine inputs",
                max: MAX_ROUTINE_KEYS,
            });
        }
        steps.sort_by_key(RoutineStep::id);

        let mut declared_inputs = BTreeSet::new();
        for input in &inputs {
            if !declared_inputs.insert(input.key().clone()) {
                return Err(RoutineError::DuplicateInput {
                    key: input.key().clone(),
                });
            }
        }

        let mut by_id: BTreeMap<RoutineStepId, &RoutineStep> = BTreeMap::new();
        for step in &steps {
            if by_id.insert(step.id(), step).is_some() {
                return Err(RoutineError::DuplicateStep { step_id: step.id() });
            }
        }
        for step in &steps {
            for dependency in step.depends_on() {
                if !by_id.contains_key(&dependency) {
                    return Err(RoutineError::UnknownDependency {
                        step_id: step.id(),
                        dependency,
                    });
                }
            }
        }

        let order = topological_order(&steps, &by_id)?;
        let ancestors = transitive_dependencies(&steps, &by_id, &order);

        for step in &steps {
            for (name, source) in step.bindings() {
                match source {
                    RoutineBindingSource::Input(key) => {
                        if !declared_inputs.contains(key) {
                            return Err(RoutineError::UnknownInputReference {
                                step_id: step.id(),
                                key: key.clone(),
                            });
                        }
                    }
                    RoutineBindingSource::StepOutput { step_id, key } => {
                        let producer =
                            by_id
                                .get(step_id)
                                .ok_or_else(|| RoutineError::UnknownDependency {
                                    step_id: step.id(),
                                    dependency: *step_id,
                                })?;
                        if !ancestors
                            .get(&step.id())
                            .is_some_and(|set| set.contains(step_id))
                        {
                            return Err(RoutineError::UnorderedOutputReference {
                                step_id: step.id(),
                                producer: *step_id,
                            });
                        }
                        if !producer.declares_output(key) {
                            return Err(RoutineError::UndeclaredOutputReference {
                                step_id: step.id(),
                                producer: *step_id,
                                key: key.clone(),
                            });
                        }
                    }
                    RoutineBindingSource::Literal(_) => {}
                }
                let _ = name;
            }
            // Rendering with placeholder-sized stand-ins would hide missing
            // bindings, so validate the placeholder names directly instead.
            for placeholder in placeholders(step.prompt().as_str(), step.id())? {
                if !step.bindings.contains_key(&placeholder) {
                    return Err(RoutineError::UnboundPlaceholder {
                        step_id: step.id(),
                        key: placeholder,
                    });
                }
            }
        }

        Ok(Self {
            id,
            routine_id,
            number,
            inputs,
            steps,
            template,
            created_at,
        })
    }

    pub const fn id(&self) -> RoutineVersionId {
        self.id
    }

    pub const fn routine_id(&self) -> RoutineId {
        self.routine_id
    }

    pub const fn number(&self) -> u32 {
        self.number
    }

    pub fn inputs(&self) -> &[RoutineInputDeclaration] {
        &self.inputs
    }

    pub fn steps(&self) -> &[RoutineStep] {
        &self.steps
    }

    pub fn step(&self, step_id: RoutineStepId) -> Option<&RoutineStep> {
        self.steps.iter().find(|step| step.id() == step_id)
    }

    pub const fn template(&self) -> Option<&Content> {
        self.template.as_ref()
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Resolves supplied values against the declared inputs, filling defaults
    /// and rejecting unknown or missing keys.
    pub fn resolve_inputs(
        &self,
        supplied: &BTreeMap<RoutineInputKey, RoutineValue>,
    ) -> Result<BTreeMap<RoutineInputKey, RoutineValue>, RoutineError> {
        let declared: BTreeSet<_> = self
            .inputs
            .iter()
            .map(|input| input.key().clone())
            .collect();
        if let Some(key) = supplied.keys().find(|key| !declared.contains(*key)) {
            return Err(RoutineError::UnknownInput { key: key.clone() });
        }
        let mut resolved = BTreeMap::new();
        for input in &self.inputs {
            let value = supplied
                .get(input.key())
                .cloned()
                .or_else(|| input.default().cloned());
            match value {
                Some(value) => {
                    resolved.insert(input.key().clone(), value);
                }
                None if input.required() => {
                    return Err(RoutineError::MissingInput {
                        key: input.key().clone(),
                    });
                }
                None => {}
            }
        }
        Ok(resolved)
    }
}

fn placeholders(
    source: &str,
    step_id: RoutineStepId,
) -> Result<Vec<RoutineInputKey>, RoutineError> {
    let mut keys = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(RoutineError::UnterminatedPlaceholder { step_id });
        };
        keys.push(RoutineInputKey::new(after[..end].trim())?);
        rest = &after[end + 2..];
    }
    Ok(keys)
}

/// Returns the steps in dependency order, rejecting cycles.
fn topological_order(
    steps: &[RoutineStep],
    by_id: &BTreeMap<RoutineStepId, &RoutineStep>,
) -> Result<Vec<RoutineStepId>, RoutineError> {
    let mut remaining: BTreeMap<RoutineStepId, usize> = steps
        .iter()
        .map(|step| (step.id(), step.depends_on.len()))
        .collect();
    let mut dependents: BTreeMap<RoutineStepId, Vec<RoutineStepId>> = BTreeMap::new();
    for step in steps {
        for dependency in step.depends_on() {
            dependents.entry(dependency).or_default().push(step.id());
        }
    }

    let mut order = Vec::with_capacity(steps.len());
    loop {
        let Some(next) = remaining
            .iter()
            .find(|(_, pending)| **pending == 0)
            .map(|(id, _)| *id)
        else {
            break;
        };
        remaining.remove(&next);
        order.push(next);
        for dependent in dependents.get(&next).into_iter().flatten() {
            if let Some(pending) = remaining.get_mut(dependent) {
                *pending -= 1;
            }
        }
    }

    if let Some((step_id, _)) = remaining.into_iter().next() {
        return Err(RoutineError::DependencyCycle { step_id });
    }
    debug_assert_eq!(order.len(), by_id.len());
    Ok(order)
}

fn transitive_dependencies(
    steps: &[RoutineStep],
    by_id: &BTreeMap<RoutineStepId, &RoutineStep>,
    order: &[RoutineStepId],
) -> BTreeMap<RoutineStepId, BTreeSet<RoutineStepId>> {
    let mut ancestors: BTreeMap<RoutineStepId, BTreeSet<RoutineStepId>> = BTreeMap::new();
    for step_id in order {
        let mut set = BTreeSet::new();
        if let Some(step) = by_id.get(step_id) {
            for dependency in step.depends_on() {
                set.insert(dependency);
                if let Some(inherited) = ancestors.get(&dependency) {
                    set.extend(inherited.iter().copied());
                }
            }
        }
        ancestors.insert(*step_id, set);
    }
    debug_assert_eq!(ancestors.len(), steps.len());
    ancestors
}

/// A named workflow and its append-only version history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    id: RoutineId,
    name: Name,
    description: Option<Content>,
    versions: Vec<RoutineVersion>,
    triggers: BTreeMap<RoutineTriggerId, RoutineTrigger>,
}

impl Routine {
    pub fn new(
        id: RoutineId,
        name: Name,
        description: Option<Content>,
        version: RoutineVersion,
    ) -> Result<Self, RoutineError> {
        if version.routine_id() != id {
            return Err(RoutineError::VersionRoutineMismatch {
                routine_id: id,
                version_id: version.id(),
            });
        }
        if version.number() != 1 {
            return Err(RoutineError::InvalidVersionNumber {
                number: version.number(),
            });
        }
        Ok(Self {
            id,
            name,
            description,
            versions: vec![version],
            triggers: BTreeMap::new(),
        })
    }

    /// Rebuilds a routine from durable storage. Version numbering is revalidated
    /// so a corrupt snapshot cannot resurrect a history that was never possible.
    pub fn restore(
        id: RoutineId,
        name: Name,
        description: Option<Content>,
        versions: Vec<RoutineVersion>,
        triggers: impl IntoIterator<Item = RoutineTrigger>,
    ) -> Result<Self, RoutineError> {
        let Some(first) = versions.first() else {
            return Err(RoutineError::EmptyRoutine { routine_id: id });
        };
        if first.number() != 1 {
            return Err(RoutineError::InvalidVersionNumber {
                number: first.number(),
            });
        }
        for (index, version) in versions.iter().enumerate() {
            if version.routine_id() != id {
                return Err(RoutineError::VersionRoutineMismatch {
                    routine_id: id,
                    version_id: version.id(),
                });
            }
            let expected = u32::try_from(index + 1).map_err(|_| RoutineError::TooMany {
                field: "routine versions",
                max: u32::MAX as usize,
            })?;
            if version.number() != expected {
                return Err(RoutineError::InvalidVersionNumber {
                    number: version.number(),
                });
            }
        }
        let mut triggers_by_id = BTreeMap::new();
        for trigger in triggers {
            if trigger.routine_id() != id {
                return Err(RoutineError::VersionRoutineMismatch {
                    routine_id: id,
                    version_id: first.id(),
                });
            }
            trigger.kind().validate()?;
            triggers_by_id.insert(trigger.id(), trigger);
        }
        Ok(Self {
            id,
            name,
            description,
            versions,
            triggers: triggers_by_id,
        })
    }

    pub const fn id(&self) -> RoutineId {
        self.id
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub const fn description(&self) -> Option<&Content> {
        self.description.as_ref()
    }

    pub fn versions(&self) -> &[RoutineVersion] {
        &self.versions
    }

    pub fn version(&self, version_id: RoutineVersionId) -> Option<&RoutineVersion> {
        self.versions
            .iter()
            .find(|version| version.id() == version_id)
    }

    pub fn latest_version(&self) -> &RoutineVersion {
        self.versions
            .last()
            .expect("a routine always keeps at least one version")
    }

    pub fn triggers(&self) -> impl Iterator<Item = &RoutineTrigger> {
        self.triggers.values()
    }

    pub fn trigger(&self, trigger_id: RoutineTriggerId) -> Option<&RoutineTrigger> {
        self.triggers.get(&trigger_id)
    }

    pub(super) fn push_version(&mut self, version: RoutineVersion) -> Result<(), RoutineError> {
        if version.routine_id() != self.id {
            return Err(RoutineError::VersionRoutineMismatch {
                routine_id: self.id,
                version_id: version.id(),
            });
        }
        let expected = self.latest_version().number().checked_add(1).ok_or(
            RoutineError::IdentifierExhausted {
                entity: "routine version",
            },
        )?;
        if version.number() != expected {
            return Err(RoutineError::InvalidVersionNumber {
                number: version.number(),
            });
        }
        if self.version(version.id()).is_some() {
            return Err(RoutineError::DuplicateVersion {
                version_id: version.id(),
            });
        }
        self.versions.push(version);
        Ok(())
    }

    pub(super) fn set_appearance(&mut self, name: Name, description: Option<Content>) {
        self.name = name;
        self.description = description;
    }

    pub(super) fn put_trigger(&mut self, trigger: RoutineTrigger) {
        self.triggers.insert(trigger.id(), trigger);
    }

    pub(super) fn remove_trigger(&mut self, trigger_id: RoutineTriggerId) -> bool {
        self.triggers.remove(&trigger_id).is_some()
    }

    pub(super) fn trigger_mut(
        &mut self,
        trigger_id: RoutineTriggerId,
    ) -> Option<&mut RoutineTrigger> {
        self.triggers.get_mut(&trigger_id)
    }
}

/// How a schedule repeats. OpenPodium has no timezone database, so a schedule
/// stores the fixed UTC offset the user selected plus a label for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineCadence {
    Hourly {
        minute: u32,
    },
    Daily {
        hour: u32,
        minute: u32,
    },
    Weekly {
        weekday: u32,
        hour: u32,
        minute: u32,
    },
}

impl RoutineCadence {
    pub const fn validate(self) -> Result<Self, RoutineError> {
        let valid = match self {
            Self::Hourly { minute } => minute < 60,
            Self::Daily { hour, minute } => hour < 24 && minute < 60,
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => weekday < 7 && hour < 24 && minute < 60,
        };
        if valid {
            Ok(self)
        } else {
            Err(RoutineError::InvalidCadence)
        }
    }

    /// Milliseconds between two firings of this cadence.
    pub const fn period_ms(self) -> u64 {
        match self {
            Self::Hourly { .. } => 3_600_000,
            Self::Daily { .. } => 86_400_000,
            Self::Weekly { .. } => 604_800_000,
        }
    }

    /// The first occurrence at or after `from`, in the schedule's local time.
    pub fn next_occurrence(self, from: Timestamp, offset_minutes: i32) -> Option<Timestamp> {
        let offset_ms = i64::from(offset_minutes).checked_mul(60_000)?;
        let local = i64::try_from(from.as_unix_millis())
            .ok()?
            .checked_add(offset_ms)?;
        let period = i64::try_from(self.period_ms()).ok()?;
        // The Unix epoch was a Thursday, so week boundaries start mid-week.
        let phase = match self {
            Self::Hourly { minute } => i64::from(minute) * 60_000,
            Self::Daily { hour, minute } => {
                i64::from(hour) * 3_600_000 + i64::from(minute) * 60_000
            }
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => {
                let days_from_thursday = (i64::from(weekday) + 3) % 7;
                days_from_thursday * 86_400_000
                    + i64::from(hour) * 3_600_000
                    + i64::from(minute) * 60_000
            }
        };
        let elapsed = local.checked_sub(phase)?;
        let periods = elapsed.div_euclid(period);
        let mut candidate = phase.checked_add(periods.checked_mul(period)?)?;
        if candidate < local {
            candidate = candidate.checked_add(period)?;
        }
        u64::try_from(candidate.checked_sub(offset_ms)?)
            .ok()
            .map(Timestamp::from_unix_millis)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineSchedule {
    cadence: RoutineCadence,
    offset_minutes: i32,
    timezone_label: Option<Name>,
    next_occurrence: Option<Timestamp>,
}

impl RoutineSchedule {
    pub const MAX_OFFSET_MINUTES: i32 = 14 * 60;

    pub fn new(
        cadence: RoutineCadence,
        offset_minutes: i32,
        timezone_label: Option<Name>,
        next_occurrence: Option<Timestamp>,
    ) -> Result<Self, RoutineError> {
        let cadence = cadence.validate()?;
        if offset_minutes.abs() > Self::MAX_OFFSET_MINUTES {
            return Err(RoutineError::InvalidTimezoneOffset { offset_minutes });
        }
        Ok(Self {
            cadence,
            offset_minutes,
            timezone_label,
            next_occurrence,
        })
    }

    pub const fn cadence(&self) -> RoutineCadence {
        self.cadence
    }

    pub const fn offset_minutes(&self) -> i32 {
        self.offset_minutes
    }

    pub const fn timezone_label(&self) -> Option<&Name> {
        self.timezone_label.as_ref()
    }

    pub const fn next_occurrence(&self) -> Option<Timestamp> {
        self.next_occurrence
    }

    pub const fn with_next_occurrence(mut self, next: Option<Timestamp>) -> Self {
        self.next_occurrence = next;
        self
    }
}

/// A local trigger source. Every trigger is observed by this process; nothing
/// fires while OpenPodium is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineTriggerKind {
    Manual,
    Filesystem {
        patterns: Vec<String>,
        debounce_ms: u64,
    },
    Git {
        refs: Vec<String>,
    },
    Schedule(RoutineSchedule),
}

impl RoutineTriggerKind {
    pub const MAX_DEBOUNCE_MS: u64 = 600_000;

    pub fn validate(&self) -> Result<(), RoutineError> {
        match self {
            Self::Manual => Ok(()),
            Self::Filesystem {
                patterns,
                debounce_ms,
            } => {
                if patterns.is_empty() || patterns.len() > MAX_TRIGGER_PATTERNS {
                    return Err(RoutineError::TooMany {
                        field: "filesystem trigger patterns",
                        max: MAX_TRIGGER_PATTERNS,
                    });
                }
                if *debounce_ms > Self::MAX_DEBOUNCE_MS {
                    return Err(RoutineError::InvalidDebounce {
                        debounce_ms: *debounce_ms,
                    });
                }
                Ok(())
            }
            Self::Git { refs } => {
                if refs.is_empty() || refs.len() > MAX_TRIGGER_PATTERNS {
                    return Err(RoutineError::TooMany {
                        field: "Git trigger refs",
                        max: MAX_TRIGGER_PATTERNS,
                    });
                }
                for reference in refs {
                    validate_ref_name(reference)?;
                }
                Ok(())
            }
            Self::Schedule(_) => Ok(()),
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::Filesystem { .. } => "Filesystem",
            Self::Git { .. } => "Git",
            Self::Schedule(_) => "Schedule",
        }
    }
}

/// Rejects the ref syntax Git itself rejects. The domain cannot reach the Git
/// boundary, so the rule is restated here and the Git module enforces the same
/// shape when it resolves a ref.
fn validate_ref_name(name: &str) -> Result<(), RoutineError> {
    let invalid = name.is_empty()
        || name.len() > 200
        || name.starts_with('-')
        || name.contains("..")
        || name.contains("@{")
        || name
            .bytes()
            .any(|byte| byte <= 32 || byte >= 127 || b"~^:?*[\\".contains(&byte))
        || name.split('/').any(|part| {
            part.is_empty()
                || part.starts_with('.')
                || part.ends_with('.')
                || part.ends_with(".lock")
        });
    if invalid {
        return Err(RoutineError::InvalidGitRef {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// The durable record of one consumed trigger occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineTriggerFiring {
    occurrence: RoutineOccurrenceKey,
    fired_at: Timestamp,
}

impl RoutineTriggerFiring {
    pub const fn new(occurrence: RoutineOccurrenceKey, fired_at: Timestamp) -> Self {
        Self {
            occurrence,
            fired_at,
        }
    }

    pub const fn occurrence(&self) -> &RoutineOccurrenceKey {
        &self.occurrence
    }

    pub const fn fired_at(&self) -> Timestamp {
        self.fired_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineTrigger {
    id: RoutineTriggerId,
    routine_id: RoutineId,
    name: Name,
    kind: RoutineTriggerKind,
    enabled: bool,
    inputs: BTreeMap<RoutineInputKey, RoutineValue>,
    last_firing: Option<RoutineTriggerFiring>,
    skipped_occurrences: u32,
}

impl RoutineTrigger {
    pub fn new(
        id: RoutineTriggerId,
        routine_id: RoutineId,
        name: Name,
        kind: RoutineTriggerKind,
        enabled: bool,
        inputs: BTreeMap<RoutineInputKey, RoutineValue>,
    ) -> Result<Self, RoutineError> {
        kind.validate()?;
        if inputs.len() > MAX_ROUTINE_KEYS {
            return Err(RoutineError::TooMany {
                field: "trigger inputs",
                max: MAX_ROUTINE_KEYS,
            });
        }
        Ok(Self {
            id,
            routine_id,
            name,
            kind,
            enabled,
            inputs,
            last_firing: None,
            skipped_occurrences: 0,
        })
    }

    pub const fn id(&self) -> RoutineTriggerId {
        self.id
    }

    pub const fn routine_id(&self) -> RoutineId {
        self.routine_id
    }

    pub const fn name(&self) -> &Name {
        &self.name
    }

    pub const fn kind(&self) -> &RoutineTriggerKind {
        &self.kind
    }

    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    pub const fn inputs(&self) -> &BTreeMap<RoutineInputKey, RoutineValue> {
        &self.inputs
    }

    pub const fn last_firing(&self) -> Option<&RoutineTriggerFiring> {
        self.last_firing.as_ref()
    }

    /// Restores the firing history persistence recorded.
    pub fn with_history(
        mut self,
        last_firing: Option<RoutineTriggerFiring>,
        skipped_occurrences: u32,
    ) -> Self {
        self.last_firing = last_firing;
        self.skipped_occurrences = skipped_occurrences;
        self
    }

    pub const fn skipped_occurrences(&self) -> u32 {
        self.skipped_occurrences
    }

    pub const fn schedule(&self) -> Option<&RoutineSchedule> {
        match &self.kind {
            RoutineTriggerKind::Schedule(schedule) => Some(schedule),
            _ => None,
        }
    }

    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Records a consumed occurrence. Firing is idempotent by occurrence key so
    /// a duplicate observation cannot create a second run.
    pub fn fire(
        &mut self,
        occurrence: RoutineOccurrenceKey,
        fired_at: Timestamp,
    ) -> Result<(), RoutineError> {
        if self
            .last_firing
            .as_ref()
            .is_some_and(|firing| firing.occurrence() == &occurrence)
        {
            return Err(RoutineError::OccurrenceAlreadyConsumed { occurrence });
        }
        self.last_firing = Some(RoutineTriggerFiring::new(occurrence, fired_at));
        Ok(())
    }

    pub fn skip(&mut self, count: u32) {
        self.skipped_occurrences = self.skipped_occurrences.saturating_add(count);
    }

    pub fn set_schedule_next(&mut self, next: Option<Timestamp>) {
        if let RoutineTriggerKind::Schedule(schedule) = &mut self.kind {
            schedule.next_occurrence = next;
        }
    }
}

/// A canonical checkout identity. Two claims conflict when these are equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoutineCheckout(WorkspaceDirectory);

impl RoutineCheckout {
    pub const fn new(directory: WorkspaceDirectory) -> Self {
        Self(directory)
    }

    pub const fn directory(&self) -> &WorkspaceDirectory {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Display for RoutineCheckout {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

/// Everything a run froze when it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineRunPin {
    version: RoutineVersion,
    inputs: BTreeMap<RoutineInputKey, RoutineValue>,
    agents: BTreeMap<RoutineStepId, AgentId>,
    checkouts: BTreeMap<RoutineStepId, RoutineCheckout>,
    revisions: BTreeMap<RoutineCheckout, String>,
    template: Option<Content>,
}

impl RoutineRunPin {
    pub fn new(
        version: RoutineVersion,
        inputs: BTreeMap<RoutineInputKey, RoutineValue>,
        agents: BTreeMap<RoutineStepId, AgentId>,
        checkouts: BTreeMap<RoutineStepId, RoutineCheckout>,
        revisions: BTreeMap<RoutineCheckout, String>,
    ) -> Result<Self, RoutineError> {
        for step in version.steps() {
            if !agents.contains_key(&step.id()) {
                return Err(RoutineError::UnpinnedStepBinding { step_id: step.id() });
            }
            if !checkouts.contains_key(&step.id()) {
                return Err(RoutineError::UnpinnedStepCheckout { step_id: step.id() });
            }
        }
        let template = version.template().cloned();
        Ok(Self {
            version,
            inputs,
            agents,
            checkouts,
            revisions,
            template,
        })
    }

    pub const fn version(&self) -> &RoutineVersion {
        &self.version
    }

    pub const fn inputs(&self) -> &BTreeMap<RoutineInputKey, RoutineValue> {
        &self.inputs
    }

    pub fn agent(&self, step_id: RoutineStepId) -> Option<AgentId> {
        self.agents.get(&step_id).copied()
    }

    pub fn checkout(&self, step_id: RoutineStepId) -> Option<&RoutineCheckout> {
        self.checkouts.get(&step_id)
    }

    pub fn agents(&self) -> impl Iterator<Item = (RoutineStepId, AgentId)> + '_ {
        self.agents.iter().map(|(step, agent)| (*step, *agent))
    }

    pub fn checkouts(&self) -> impl Iterator<Item = (RoutineStepId, &RoutineCheckout)> {
        self.checkouts
            .iter()
            .map(|(step, checkout)| (*step, checkout))
    }

    pub const fn revisions(&self) -> &BTreeMap<RoutineCheckout, String> {
        &self.revisions
    }

    pub const fn template(&self) -> Option<&Content> {
        self.template.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineRunState {
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl RoutineRunState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

impl Display for RoutineRunState {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineStepState {
    /// Dependencies are unmet, or the step is ready but not yet dispatched.
    Pending,
    /// The step is otherwise ready and waits for a human decision.
    AwaitingApproval,
    /// Work is dispatched; a task and handoff exist.
    Dispatched,
    Completed,
    Failed,
    Cancelled,
    /// A dependency did not complete, so this step will never run.
    Skipped,
    /// Work was dispatched but its outcome is unknown. A user must resolve it.
    Interrupted,
}

impl RoutineStepState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Skipped
        )
    }

    /// Whether the step still holds the resources its attempt acquired.
    pub const fn holds_reservations(self) -> bool {
        matches!(self, Self::Dispatched | Self::Interrupted)
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::AwaitingApproval => "awaiting approval",
            Self::Dispatched => "dispatched",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
            Self::Interrupted => "interrupted",
        }
    }
}

impl Display for RoutineStepState {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineInterruptionReason {
    /// OpenPodium restarted while the attempt's outcome was unknown.
    DispatchOutcomeUnknown,
    /// Cancellation was requested but the agent never confirmed it stopped.
    ShutdownUnconfirmed,
}

impl RoutineInterruptionReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DispatchOutcomeUnknown => {
                "the step was dispatched and its outcome is unknown after a restart"
            }
            Self::ShutdownUnconfirmed => {
                "cancellation was requested but the agent has not confirmed it stopped"
            }
        }
    }
}

impl Display for RoutineInterruptionReason {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineInterruption {
    reason: RoutineInterruptionReason,
    detected_at: Timestamp,
}

impl RoutineInterruption {
    pub const fn new(reason: RoutineInterruptionReason, detected_at: Timestamp) -> Self {
        Self {
            reason,
            detected_at,
        }
    }

    pub const fn reason(&self) -> RoutineInterruptionReason {
        self.reason
    }

    pub const fn detected_at(&self) -> Timestamp {
        self.detected_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineApprovalDecision {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineApprovalRecord {
    decision: RoutineApprovalDecision,
    decided_at: Timestamp,
    note: Option<Content>,
}

impl RoutineApprovalRecord {
    pub const fn new(
        decision: RoutineApprovalDecision,
        decided_at: Timestamp,
        note: Option<Content>,
    ) -> Self {
        Self {
            decision,
            decided_at,
            note,
        }
    }

    pub const fn decision(&self) -> RoutineApprovalDecision {
        self.decision
    }

    pub const fn decided_at(&self) -> Timestamp {
        self.decided_at
    }

    pub const fn note(&self) -> Option<&Content> {
        self.note.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineAttemptOutcome {
    Completed,
    Failed,
    Cancelled,
}

/// One dispatch of one step, including the reservations it acquired. Retries
/// append an attempt; they never replace the previous task or handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineAttempt {
    id: RoutineAttemptId,
    ordinal: u32,
    task_id: TaskId,
    handoff_id: HandoffId,
    agent_id: AgentId,
    checkout: RoutineCheckout,
    resources: BTreeSet<RoutineResourceKey>,
    started_at: Timestamp,
    outcome: Option<RoutineAttemptOutcome>,
    finished_at: Option<Timestamp>,
}

impl RoutineAttempt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: RoutineAttemptId,
        ordinal: u32,
        task_id: TaskId,
        handoff_id: HandoffId,
        agent_id: AgentId,
        checkout: RoutineCheckout,
        resources: impl IntoIterator<Item = RoutineResourceKey>,
        started_at: Timestamp,
    ) -> Result<Self, RoutineError> {
        if ordinal == 0 {
            return Err(RoutineError::InvalidAttemptOrdinal { ordinal });
        }
        Ok(Self {
            id,
            ordinal,
            task_id,
            handoff_id,
            agent_id,
            checkout,
            resources: resources.into_iter().collect(),
            started_at,
            outcome: None,
            finished_at: None,
        })
    }

    pub const fn id(&self) -> RoutineAttemptId {
        self.id
    }

    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub const fn task_id(&self) -> TaskId {
        self.task_id
    }

    pub const fn handoff_id(&self) -> HandoffId {
        self.handoff_id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub const fn checkout(&self) -> &RoutineCheckout {
        &self.checkout
    }

    pub fn resources(&self) -> impl Iterator<Item = &RoutineResourceKey> {
        self.resources.iter()
    }

    pub const fn started_at(&self) -> Timestamp {
        self.started_at
    }

    pub const fn outcome(&self) -> Option<RoutineAttemptOutcome> {
        self.outcome
    }

    pub const fn finished_at(&self) -> Option<Timestamp> {
        self.finished_at
    }

    /// Restores a finished attempt from durable storage.
    pub fn with_outcome(
        mut self,
        outcome: RoutineAttemptOutcome,
        finished_at: Timestamp,
    ) -> Result<Self, RoutineError> {
        self.finish(outcome, finished_at)?;
        Ok(self)
    }

    fn finish(
        &mut self,
        outcome: RoutineAttemptOutcome,
        finished_at: Timestamp,
    ) -> Result<(), RoutineError> {
        if self.outcome.is_some() {
            return Err(RoutineError::AttemptAlreadyFinished {
                ordinal: self.ordinal,
            });
        }
        if finished_at < self.started_at {
            return Err(RoutineError::InvalidAttemptTime {
                ordinal: self.ordinal,
            });
        }
        self.outcome = Some(outcome);
        self.finished_at = Some(finished_at);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineStepRun {
    step_id: RoutineStepId,
    state: RoutineStepState,
    attempts: Vec<RoutineAttempt>,
    outputs: BTreeMap<RoutineOutputKey, RoutineValue>,
    approval: Option<RoutineApprovalRecord>,
    interruption: Option<RoutineInterruption>,
    failure: Option<Content>,
}

impl RoutineStepRun {
    const fn pending(step_id: RoutineStepId) -> Self {
        Self {
            step_id,
            state: RoutineStepState::Pending,
            attempts: Vec::new(),
            outputs: BTreeMap::new(),
            approval: None,
            interruption: None,
            failure: None,
        }
    }

    /// Rebuilds one step's durable state. Attempt ordinals are revalidated so a
    /// corrupt record cannot claim a retry history the policy never allowed.
    pub fn restore(
        step_id: RoutineStepId,
        state: RoutineStepState,
        attempts: Vec<RoutineAttempt>,
        outputs: BTreeMap<RoutineOutputKey, RoutineValue>,
        approval: Option<RoutineApprovalRecord>,
        interruption: Option<RoutineInterruption>,
        failure: Option<Content>,
    ) -> Result<Self, RoutineError> {
        for (index, attempt) in attempts.iter().enumerate() {
            let expected = u32::try_from(index + 1).unwrap_or(u32::MAX);
            if attempt.ordinal() != expected {
                return Err(RoutineError::InvalidAttemptOrdinal {
                    ordinal: attempt.ordinal(),
                });
            }
        }
        if state.holds_reservations() && attempts.last().is_none_or(|a| a.outcome().is_some()) {
            return Err(RoutineError::NoAttempt { step_id });
        }
        if state == RoutineStepState::Interrupted && interruption.is_none() {
            return Err(RoutineError::InvalidStepTransition {
                step_id,
                from: state,
                action: "restore",
            });
        }
        Ok(Self {
            step_id,
            state,
            attempts,
            outputs,
            approval,
            interruption,
            failure,
        })
    }

    pub const fn step_id(&self) -> RoutineStepId {
        self.step_id
    }

    pub const fn state(&self) -> RoutineStepState {
        self.state
    }

    pub fn attempts(&self) -> &[RoutineAttempt] {
        &self.attempts
    }

    pub fn active_attempt(&self) -> Option<&RoutineAttempt> {
        self.attempts
            .last()
            .filter(|attempt| attempt.outcome().is_none())
    }

    pub const fn outputs(&self) -> &BTreeMap<RoutineOutputKey, RoutineValue> {
        &self.outputs
    }

    pub const fn approval(&self) -> Option<&RoutineApprovalRecord> {
        self.approval.as_ref()
    }

    pub const fn interruption(&self) -> Option<&RoutineInterruption> {
        self.interruption.as_ref()
    }

    pub const fn failure(&self) -> Option<&Content> {
        self.failure.as_ref()
    }
}

/// One execution of a pinned routine version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineRun {
    id: RoutineRunId,
    routine_id: RoutineId,
    version_id: RoutineVersionId,
    trigger_id: Option<RoutineTriggerId>,
    occurrence: Option<RoutineOccurrenceKey>,
    pin: RoutineRunPin,
    state: RoutineRunState,
    started_at: Timestamp,
    finished_at: Option<Timestamp>,
    steps: BTreeMap<RoutineStepId, RoutineStepRun>,
}

impl RoutineRun {
    pub fn start(
        id: RoutineRunId,
        trigger_id: Option<RoutineTriggerId>,
        occurrence: Option<RoutineOccurrenceKey>,
        pin: RoutineRunPin,
        started_at: Timestamp,
    ) -> Self {
        let steps = pin
            .version()
            .steps()
            .iter()
            .map(|step| (step.id(), RoutineStepRun::pending(step.id())))
            .collect();
        Self {
            id,
            routine_id: pin.version().routine_id(),
            version_id: pin.version().id(),
            trigger_id,
            occurrence,
            pin,
            state: RoutineRunState::Running,
            started_at,
            finished_at: None,
            steps,
        }
    }

    /// Rebuilds a run from durable storage. The pinned version is the authority
    /// for which steps exist, so a record that describes a different step set is
    /// rejected rather than silently reshaped.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: RoutineRunId,
        trigger_id: Option<RoutineTriggerId>,
        occurrence: Option<RoutineOccurrenceKey>,
        pin: RoutineRunPin,
        state: RoutineRunState,
        started_at: Timestamp,
        finished_at: Option<Timestamp>,
        steps: Vec<RoutineStepRun>,
    ) -> Result<Self, RoutineError> {
        let steps: BTreeMap<_, _> = steps
            .into_iter()
            .map(|step| (step.step_id(), step))
            .collect();
        for step in pin.version().steps() {
            if !steps.contains_key(&step.id()) {
                return Err(RoutineError::UnknownStep { step_id: step.id() });
            }
        }
        if steps.len() != pin.version().steps().len() {
            let unknown = steps
                .keys()
                .copied()
                .find(|step_id| pin.version().step(*step_id).is_none())
                .expect("the step counts differ, so one step is not in the version");
            return Err(RoutineError::UnknownStep { step_id: unknown });
        }
        if state.is_terminal() != finished_at.is_some() {
            return Err(RoutineError::InvalidRunTransition {
                run_id: id,
                from: state,
                action: "restore",
            });
        }
        Ok(Self {
            id,
            routine_id: pin.version().routine_id(),
            version_id: pin.version().id(),
            trigger_id,
            occurrence,
            pin,
            state,
            started_at,
            finished_at,
            steps,
        })
    }

    pub const fn id(&self) -> RoutineRunId {
        self.id
    }

    pub const fn routine_id(&self) -> RoutineId {
        self.routine_id
    }

    pub const fn version_id(&self) -> RoutineVersionId {
        self.version_id
    }

    pub const fn trigger_id(&self) -> Option<RoutineTriggerId> {
        self.trigger_id
    }

    pub const fn occurrence(&self) -> Option<&RoutineOccurrenceKey> {
        self.occurrence.as_ref()
    }

    pub const fn pin(&self) -> &RoutineRunPin {
        &self.pin
    }

    pub const fn state(&self) -> RoutineRunState {
        self.state
    }

    pub const fn started_at(&self) -> Timestamp {
        self.started_at
    }

    pub const fn finished_at(&self) -> Option<Timestamp> {
        self.finished_at
    }

    pub fn steps(&self) -> impl Iterator<Item = &RoutineStepRun> {
        self.steps.values()
    }

    pub fn step(&self, step_id: RoutineStepId) -> Option<&RoutineStepRun> {
        self.steps.get(&step_id)
    }

    pub fn is_active(&self) -> bool {
        !self.state.is_terminal()
    }

    /// The step run that owns `task_id`, if any.
    pub fn step_for_task(&self, task_id: TaskId) -> Option<&RoutineStepRun> {
        self.steps.values().find(|step| {
            step.attempts
                .iter()
                .any(|attempt| attempt.task_id() == task_id)
        })
    }

    /// Reservations this run currently holds. A dispatched step holds them
    /// until its execution ends; an interrupted step keeps holding them until
    /// the user resolves it, because a cancelled handoff does not prove the
    /// agent stopped touching the checkout.
    pub fn held_reservations(&self) -> Vec<RoutineReservation> {
        self.steps
            .values()
            .filter(|step| step.state().holds_reservations())
            .filter_map(RoutineStepRun::active_or_last_attempt)
            .flat_map(|attempt| {
                let mut claims = vec![
                    RoutineReservation::Agent(attempt.agent_id()),
                    RoutineReservation::Checkout(attempt.checkout().clone()),
                ];
                claims.extend(attempt.resources().cloned().map(RoutineReservation::Named));
                claims
            })
            .collect()
    }

    /// Steps whose dependencies all completed and which are not yet dispatched,
    /// in stable step order.
    pub fn ready_steps(&self) -> Vec<RoutineStepId> {
        self.steps
            .values()
            .filter(|step| matches!(step.state(), RoutineStepState::Pending))
            .filter(|step| self.dependencies_completed(step.step_id()))
            .map(RoutineStepRun::step_id)
            .collect()
    }

    fn dependencies_completed(&self, step_id: RoutineStepId) -> bool {
        self.pin.version().step(step_id).is_some_and(|step| {
            step.depends_on().all(|dependency| {
                self.steps
                    .get(&dependency)
                    .is_some_and(|run| run.state() == RoutineStepState::Completed)
            })
        })
    }

    /// Resolves a step's bindings against the pinned inputs and the outputs its
    /// dependencies already produced.
    pub fn resolve_bindings(
        &self,
        step_id: RoutineStepId,
    ) -> Result<BTreeMap<RoutineInputKey, RoutineValue>, RoutineError> {
        let step = self
            .pin
            .version()
            .step(step_id)
            .ok_or(RoutineError::UnknownStep { step_id })?;
        let mut resolved = BTreeMap::new();
        for (name, source) in step.bindings() {
            let value = match source {
                RoutineBindingSource::Input(key) => self
                    .pin
                    .inputs()
                    .get(key)
                    .cloned()
                    .ok_or_else(|| RoutineError::MissingInput { key: key.clone() })?,
                RoutineBindingSource::StepOutput {
                    step_id: producer,
                    key,
                } => self
                    .steps
                    .get(producer)
                    .and_then(|run| run.outputs().get(key))
                    .cloned()
                    .ok_or_else(|| RoutineError::MissingStepOutput {
                        step_id,
                        producer: *producer,
                        key: key.clone(),
                    })?,
                RoutineBindingSource::Literal(value) => value.clone(),
            };
            resolved.insert(name.clone(), value);
        }
        Ok(resolved)
    }

    pub fn require_approval(&mut self, step_id: RoutineStepId) -> Result<(), RoutineError> {
        let step = self.step_mut(step_id)?;
        expect_state(step, RoutineStepState::Pending, "await approval")?;
        step.state = RoutineStepState::AwaitingApproval;
        Ok(())
    }

    pub fn record_approval(
        &mut self,
        step_id: RoutineStepId,
        record: RoutineApprovalRecord,
    ) -> Result<(), RoutineError> {
        let step = self.step_mut(step_id)?;
        if !matches!(
            step.state,
            RoutineStepState::Pending | RoutineStepState::AwaitingApproval
        ) {
            return Err(RoutineError::InvalidStepTransition {
                step_id,
                from: step.state,
                action: "approve",
            });
        }
        if step.approval.is_some() {
            return Err(RoutineError::ApprovalAlreadyRecorded { step_id });
        }
        let decision = record.decision();
        step.approval = Some(record);
        match decision {
            RoutineApprovalDecision::Approved => step.state = RoutineStepState::Pending,
            RoutineApprovalDecision::Rejected => {
                step.state = RoutineStepState::Cancelled;
                self.propagate_skips();
            }
        }
        Ok(())
    }

    pub fn dispatch(
        &mut self,
        step_id: RoutineStepId,
        attempt: RoutineAttempt,
    ) -> Result<(), RoutineError> {
        let max_attempts = self
            .pin
            .version()
            .step(step_id)
            .ok_or(RoutineError::UnknownStep { step_id })?
            .retry()
            .max_attempts();
        if self.state != RoutineRunState::Running {
            return Err(RoutineError::InvalidRunTransition {
                run_id: self.id,
                from: self.state,
                action: "dispatch",
            });
        }
        if !self.dependencies_completed(step_id) {
            return Err(RoutineError::DependenciesUnmet { step_id });
        }
        let step = self.step_mut(step_id)?;
        expect_state(step, RoutineStepState::Pending, "dispatch")?;
        let expected = u32::try_from(step.attempts.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or(RoutineError::IdentifierExhausted {
                entity: "routine attempt",
            })?;
        if attempt.ordinal() != expected {
            return Err(RoutineError::InvalidAttemptOrdinal {
                ordinal: attempt.ordinal(),
            });
        }
        if expected > max_attempts {
            return Err(RoutineError::RetryBudgetExhausted {
                step_id,
                max_attempts,
            });
        }
        step.attempts.push(attempt);
        step.state = RoutineStepState::Dispatched;
        Ok(())
    }

    pub fn complete_step(
        &mut self,
        step_id: RoutineStepId,
        outputs: BTreeMap<RoutineOutputKey, RoutineValue>,
        finished_at: Timestamp,
    ) -> Result<(), RoutineError> {
        let declared: BTreeSet<_> = self
            .pin
            .version()
            .step(step_id)
            .ok_or(RoutineError::UnknownStep { step_id })?
            .outputs()
            .cloned()
            .collect();
        if let Some(key) = outputs.keys().find(|key| !declared.contains(*key)) {
            return Err(RoutineError::UndeclaredOutput {
                step_id,
                key: key.clone(),
            });
        }
        if let Some(key) = declared.iter().find(|key| !outputs.contains_key(*key)) {
            return Err(RoutineError::MissingOutput {
                step_id,
                key: key.clone(),
            });
        }
        let step = self.step_mut(step_id)?;
        expect_state(step, RoutineStepState::Dispatched, "complete")?;
        finish_attempt(step, RoutineAttemptOutcome::Completed, finished_at)?;
        step.outputs = outputs;
        step.state = RoutineStepState::Completed;
        Ok(())
    }

    pub fn fail_step(
        &mut self,
        step_id: RoutineStepId,
        reason: Content,
        finished_at: Timestamp,
    ) -> Result<bool, RoutineError> {
        let max_attempts = self
            .pin
            .version()
            .step(step_id)
            .ok_or(RoutineError::UnknownStep { step_id })?
            .retry()
            .max_attempts();
        let step = self.step_mut(step_id)?;
        if !matches!(
            step.state,
            RoutineStepState::Dispatched | RoutineStepState::Interrupted
        ) {
            return Err(RoutineError::InvalidStepTransition {
                step_id,
                from: step.state,
                action: "fail",
            });
        }
        finish_attempt(step, RoutineAttemptOutcome::Failed, finished_at)?;
        step.failure = Some(reason);
        step.interruption = None;
        let attempts = u32::try_from(step.attempts.len()).unwrap_or(u32::MAX);
        let retryable = attempts < max_attempts;
        step.state = if retryable {
            RoutineStepState::Pending
        } else {
            RoutineStepState::Failed
        };
        if !retryable {
            self.propagate_skips();
        }
        Ok(retryable)
    }

    pub fn interrupt_step(
        &mut self,
        step_id: RoutineStepId,
        reason: RoutineInterruptionReason,
        detected_at: Timestamp,
    ) -> Result<(), RoutineError> {
        let step = self.step_mut(step_id)?;
        expect_state(step, RoutineStepState::Dispatched, "interrupt")?;
        step.state = RoutineStepState::Interrupted;
        step.interruption = Some(RoutineInterruption::new(reason, detected_at));
        Ok(())
    }

    /// Clears an interruption after the user confirmed the work stopped. The
    /// step becomes retryable when its budget allows, and fails otherwise.
    pub fn resolve_interruption(
        &mut self,
        step_id: RoutineStepId,
        finished_at: Timestamp,
    ) -> Result<(), RoutineError> {
        let max_attempts = self
            .pin
            .version()
            .step(step_id)
            .ok_or(RoutineError::UnknownStep { step_id })?
            .retry()
            .max_attempts();
        // A run that is cancelling does not retry the work it just confirmed
        // stopped. Returning the step to pending would leave a cancelled run
        // active forever: it never dispatches again, and it never settles.
        let cancelling = self.state == RoutineRunState::Cancelling;
        let step = self.step_mut(step_id)?;
        expect_state(step, RoutineStepState::Interrupted, "resolve")?;
        finish_attempt(step, RoutineAttemptOutcome::Cancelled, finished_at)?;
        step.interruption = None;
        let attempts = u32::try_from(step.attempts.len()).unwrap_or(u32::MAX);
        step.state = if cancelling {
            RoutineStepState::Cancelled
        } else if attempts < max_attempts {
            RoutineStepState::Pending
        } else {
            RoutineStepState::Failed
        };
        if matches!(
            step.state,
            RoutineStepState::Failed | RoutineStepState::Cancelled
        ) {
            self.propagate_skips();
        }
        Ok(())
    }

    pub fn cancel_step(
        &mut self,
        step_id: RoutineStepId,
        finished_at: Timestamp,
    ) -> Result<(), RoutineError> {
        let step = self.step_mut(step_id)?;
        if step.state.is_terminal() {
            return Ok(());
        }
        if matches!(step.state, RoutineStepState::Dispatched) {
            finish_attempt(step, RoutineAttemptOutcome::Cancelled, finished_at)?;
        }
        step.state = RoutineStepState::Cancelled;
        self.propagate_skips();
        Ok(())
    }

    /// Requests cancellation. Dispatched steps stay dispatched until their
    /// cancellation is confirmed, so their reservations remain held.
    pub fn request_cancellation(&mut self, at: Timestamp) -> Result<(), RoutineError> {
        if self.state.is_terminal() {
            return Err(RoutineError::InvalidRunTransition {
                run_id: self.id,
                from: self.state,
                action: "cancel",
            });
        }
        for step_id in self.steps.keys().copied().collect::<Vec<_>>() {
            let step = self
                .steps
                .get_mut(&step_id)
                .expect("the step identifier came from this run");
            if matches!(
                step.state,
                RoutineStepState::Pending | RoutineStepState::AwaitingApproval
            ) {
                step.state = RoutineStepState::Cancelled;
            }
        }
        self.state = RoutineRunState::Cancelling;
        let _ = at;
        Ok(())
    }

    /// Recomputes the run state from its steps. Returns true when it changed.
    pub fn settle(&mut self, at: Timestamp) -> bool {
        if self.state.is_terminal() {
            return false;
        }
        let interrupted = self
            .steps
            .values()
            .any(|step| step.state == RoutineStepState::Interrupted);
        let busy = self
            .steps
            .values()
            .any(|step| matches!(step.state, RoutineStepState::Dispatched));
        let waiting = self.steps.values().any(|step| {
            matches!(
                step.state,
                RoutineStepState::Pending | RoutineStepState::AwaitingApproval
            )
        });
        if busy || interrupted || (waiting && self.state == RoutineRunState::Running) {
            return false;
        }
        let cancelled = self.state == RoutineRunState::Cancelling
            || self
                .steps
                .values()
                .any(|step| step.state == RoutineStepState::Cancelled);
        let failed = self.steps.values().any(|step| {
            matches!(
                step.state,
                RoutineStepState::Failed | RoutineStepState::Skipped
            )
        });
        self.state = if failed {
            RoutineRunState::Failed
        } else if cancelled {
            RoutineRunState::Cancelled
        } else {
            RoutineRunState::Completed
        };
        self.finished_at = Some(at);
        true
    }

    fn propagate_skips(&mut self) {
        loop {
            let blocked: Vec<RoutineStepId> = self
                .steps
                .values()
                .filter(|run| {
                    matches!(
                        run.state,
                        RoutineStepState::Pending | RoutineStepState::AwaitingApproval
                    )
                })
                .filter(|run| {
                    self.pin.version().step(run.step_id()).is_some_and(|step| {
                        step.depends_on().any(|dependency| {
                            self.steps.get(&dependency).is_some_and(|other| {
                                matches!(
                                    other.state,
                                    RoutineStepState::Failed
                                        | RoutineStepState::Cancelled
                                        | RoutineStepState::Skipped
                                )
                            })
                        })
                    })
                })
                .map(RoutineStepRun::step_id)
                .collect();
            if blocked.is_empty() {
                return;
            }
            for step_id in blocked {
                if let Some(step) = self.steps.get_mut(&step_id) {
                    step.state = RoutineStepState::Skipped;
                }
            }
        }
    }

    fn step_mut(&mut self, step_id: RoutineStepId) -> Result<&mut RoutineStepRun, RoutineError> {
        self.steps
            .get_mut(&step_id)
            .ok_or(RoutineError::UnknownStep { step_id })
    }
}

impl RoutineStepRun {
    fn active_or_last_attempt(&self) -> Option<&RoutineAttempt> {
        self.attempts.last()
    }
}

fn expect_state(
    step: &RoutineStepRun,
    expected: RoutineStepState,
    action: &'static str,
) -> Result<(), RoutineError> {
    if step.state == expected {
        Ok(())
    } else {
        Err(RoutineError::InvalidStepTransition {
            step_id: step.step_id,
            from: step.state,
            action,
        })
    }
}

fn finish_attempt(
    step: &mut RoutineStepRun,
    outcome: RoutineAttemptOutcome,
    finished_at: Timestamp,
) -> Result<(), RoutineError> {
    let step_id = step.step_id;
    step.attempts
        .last_mut()
        .ok_or(RoutineError::NoAttempt { step_id })?
        .finish(outcome, finished_at)
}

/// One atomic change to a run that does not create a task or handoff.
///
/// Dispatch is deliberately absent: it must record the attempt, the task, and
/// the handoff in the same journal batch, so it has its own command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineTransition {
    AwaitApproval {
        step_id: RoutineStepId,
    },
    RecordApproval {
        step_id: RoutineStepId,
        record: RoutineApprovalRecord,
    },
    CompleteStep {
        step_id: RoutineStepId,
        outputs: BTreeMap<RoutineOutputKey, RoutineValue>,
        finished_at: Timestamp,
    },
    FailStep {
        step_id: RoutineStepId,
        reason: Content,
        finished_at: Timestamp,
    },
    InterruptStep {
        step_id: RoutineStepId,
        reason: RoutineInterruptionReason,
        detected_at: Timestamp,
    },
    ResolveInterruption {
        step_id: RoutineStepId,
        finished_at: Timestamp,
    },
    CancelStep {
        step_id: RoutineStepId,
        finished_at: Timestamp,
    },
    RequestCancellation {
        at: Timestamp,
    },
    Settle {
        at: Timestamp,
    },
}

impl RoutineRun {
    /// Applies one transition. Replay and live execution share this path so a
    /// journalled transition can never bypass the run's invariants.
    pub fn apply_transition(&mut self, transition: &RoutineTransition) -> Result<(), RoutineError> {
        match transition {
            RoutineTransition::AwaitApproval { step_id } => self.require_approval(*step_id),
            RoutineTransition::RecordApproval { step_id, record } => {
                self.record_approval(*step_id, record.clone())
            }
            RoutineTransition::CompleteStep {
                step_id,
                outputs,
                finished_at,
            } => self.complete_step(*step_id, outputs.clone(), *finished_at),
            RoutineTransition::FailStep {
                step_id,
                reason,
                finished_at,
            } => self
                .fail_step(*step_id, reason.clone(), *finished_at)
                .map(|_| ()),
            RoutineTransition::InterruptStep {
                step_id,
                reason,
                detected_at,
            } => self.interrupt_step(*step_id, *reason, *detected_at),
            RoutineTransition::ResolveInterruption {
                step_id,
                finished_at,
            } => self.resolve_interruption(*step_id, *finished_at),
            RoutineTransition::CancelStep {
                step_id,
                finished_at,
            } => self.cancel_step(*step_id, *finished_at),
            RoutineTransition::RequestCancellation { at } => self.request_cancellation(*at),
            RoutineTransition::Settle { at } => {
                if self.settle(*at) {
                    Ok(())
                } else {
                    Err(RoutineError::InvalidRunTransition {
                        run_id: self.id,
                        from: self.state,
                        action: "settle",
                    })
                }
            }
        }
    }
}

/// A claim the scheduler must acquire before it dispatches a step.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RoutineReservation {
    Agent(AgentId),
    Checkout(RoutineCheckout),
    Named(RoutineResourceKey),
}

impl Display for RoutineReservation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Agent(id) => write!(formatter, "agent {id}"),
            Self::Checkout(checkout) => write!(formatter, "checkout {checkout}"),
            Self::Named(key) => write!(formatter, "resource {key}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineError {
    InvalidKey {
        field: &'static str,
    },
    ValueTooLong {
        max_chars: usize,
    },
    TooMany {
        field: &'static str,
        max: usize,
    },
    EmptyRoutine {
        routine_id: RoutineId,
    },
    DuplicateStep {
        step_id: RoutineStepId,
    },
    DuplicateInput {
        key: RoutineInputKey,
    },
    DuplicateVersion {
        version_id: RoutineVersionId,
    },
    UnknownDependency {
        step_id: RoutineStepId,
        dependency: RoutineStepId,
    },
    DependencyCycle {
        step_id: RoutineStepId,
    },
    UnknownInputReference {
        step_id: RoutineStepId,
        key: RoutineInputKey,
    },
    UnorderedOutputReference {
        step_id: RoutineStepId,
        producer: RoutineStepId,
    },
    UndeclaredOutputReference {
        step_id: RoutineStepId,
        producer: RoutineStepId,
        key: RoutineOutputKey,
    },
    UnterminatedPlaceholder {
        step_id: RoutineStepId,
    },
    UnboundPlaceholder {
        step_id: RoutineStepId,
        key: RoutineInputKey,
    },
    EmptyRenderedPrompt {
        step_id: RoutineStepId,
    },
    InvalidVersionNumber {
        number: u32,
    },
    VersionRoutineMismatch {
        routine_id: RoutineId,
        version_id: RoutineVersionId,
    },
    InvalidRetryPolicy,
    InvalidCadence,
    InvalidTimezoneOffset {
        offset_minutes: i32,
    },
    InvalidDebounce {
        debounce_ms: u64,
    },
    InvalidGitRef {
        name: String,
    },
    OccurrenceAlreadyConsumed {
        occurrence: RoutineOccurrenceKey,
    },
    UnknownInput {
        key: RoutineInputKey,
    },
    MissingInput {
        key: RoutineInputKey,
    },
    UnpinnedStepBinding {
        step_id: RoutineStepId,
    },
    UnpinnedStepCheckout {
        step_id: RoutineStepId,
    },
    UnknownStep {
        step_id: RoutineStepId,
    },
    MissingStepOutput {
        step_id: RoutineStepId,
        producer: RoutineStepId,
        key: RoutineOutputKey,
    },
    UndeclaredOutput {
        step_id: RoutineStepId,
        key: RoutineOutputKey,
    },
    MissingOutput {
        step_id: RoutineStepId,
        key: RoutineOutputKey,
    },
    DependenciesUnmet {
        step_id: RoutineStepId,
    },
    ApprovalAlreadyRecorded {
        step_id: RoutineStepId,
    },
    InvalidStepTransition {
        step_id: RoutineStepId,
        from: RoutineStepState,
        action: &'static str,
    },
    InvalidRunTransition {
        run_id: RoutineRunId,
        from: RoutineRunState,
        action: &'static str,
    },
    InvalidAttemptOrdinal {
        ordinal: u32,
    },
    InvalidAttemptTime {
        ordinal: u32,
    },
    AttemptAlreadyFinished {
        ordinal: u32,
    },
    NoAttempt {
        step_id: RoutineStepId,
    },
    RetryBudgetExhausted {
        step_id: RoutineStepId,
        max_attempts: u32,
    },
    IdentifierExhausted {
        entity: &'static str,
    },
}

impl Display for RoutineError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey { field } => write!(
                formatter,
                "{field} must use letters, digits, dots, dashes, or underscores"
            ),
            Self::ValueTooLong { max_chars } => {
                write!(formatter, "value cannot exceed {max_chars} characters")
            }
            Self::TooMany { field, max } => {
                write!(formatter, "{field} cannot exceed {max} entries")
            }
            Self::EmptyRoutine { routine_id } => {
                write!(formatter, "routine {routine_id} must declare a step")
            }
            Self::DuplicateStep { step_id } => {
                write!(formatter, "step {step_id} is declared more than once")
            }
            Self::DuplicateInput { key } => {
                write!(formatter, "input {key} is declared more than once")
            }
            Self::DuplicateVersion { version_id } => {
                write!(formatter, "routine version {version_id} already exists")
            }
            Self::UnknownDependency {
                step_id,
                dependency,
            } => write!(
                formatter,
                "step {step_id} depends on step {dependency}, which the routine does not declare"
            ),
            Self::DependencyCycle { step_id } => write!(
                formatter,
                "step {step_id} takes part in a dependency cycle; routine steps must form a DAG"
            ),
            Self::UnknownInputReference { step_id, key } => write!(
                formatter,
                "step {step_id} reads input {key}, which the routine does not declare"
            ),
            Self::UnorderedOutputReference { step_id, producer } => write!(
                formatter,
                "step {step_id} reads an output of step {producer} without depending on it"
            ),
            Self::UndeclaredOutputReference {
                step_id,
                producer,
                key,
            } => write!(
                formatter,
                "step {step_id} reads output {key}, which step {producer} does not declare"
            ),
            Self::UnterminatedPlaceholder { step_id } => write!(
                formatter,
                "step {step_id} has a prompt placeholder that is never closed"
            ),
            Self::UnboundPlaceholder { step_id, key } => write!(
                formatter,
                "step {step_id} uses placeholder {key}, which it does not bind"
            ),
            Self::EmptyRenderedPrompt { step_id } => {
                write!(formatter, "step {step_id} renders an empty prompt")
            }
            Self::InvalidVersionNumber { number } => write!(
                formatter,
                "routine version {number} does not follow the stored history"
            ),
            Self::VersionRoutineMismatch {
                routine_id,
                version_id,
            } => write!(
                formatter,
                "version {version_id} does not belong to routine {routine_id}"
            ),
            Self::InvalidRetryPolicy => {
                formatter.write_str("a retry policy must allow at least one attempt")
            }
            Self::InvalidCadence => formatter.write_str("the schedule cadence is out of range"),
            Self::InvalidTimezoneOffset { offset_minutes } => write!(
                formatter,
                "timezone offset {offset_minutes} is outside the supported range"
            ),
            Self::InvalidDebounce { debounce_ms } => {
                write!(formatter, "debounce {debounce_ms}ms is too long")
            }
            Self::InvalidGitRef { name } => write!(formatter, "{name:?} is not a valid Git ref"),
            Self::OccurrenceAlreadyConsumed { occurrence } => write!(
                formatter,
                "trigger occurrence {occurrence} was already consumed"
            ),
            Self::UnknownInput { key } => {
                write!(formatter, "the routine does not declare input {key}")
            }
            Self::MissingInput { key } => write!(formatter, "input {key} is required"),
            Self::UnpinnedStepBinding { step_id } => write!(
                formatter,
                "the run did not pin an agent binding for step {step_id}"
            ),
            Self::UnpinnedStepCheckout { step_id } => write!(
                formatter,
                "the run did not pin a checkout for step {step_id}"
            ),
            Self::UnknownStep { step_id } => {
                write!(formatter, "step {step_id} is not part of this run")
            }
            Self::MissingStepOutput {
                step_id,
                producer,
                key,
            } => write!(
                formatter,
                "step {step_id} needs output {key} from step {producer}, which has not produced it"
            ),
            Self::UndeclaredOutput { step_id, key } => write!(
                formatter,
                "step {step_id} returned output {key}, which it does not declare"
            ),
            Self::MissingOutput { step_id, key } => write!(
                formatter,
                "step {step_id} did not return its declared output {key}"
            ),
            Self::DependenciesUnmet { step_id } => write!(
                formatter,
                "step {step_id} cannot start before its dependencies complete"
            ),
            Self::ApprovalAlreadyRecorded { step_id } => {
                write!(formatter, "step {step_id} already recorded an approval")
            }
            Self::InvalidStepTransition {
                step_id,
                from,
                action,
            } => write!(
                formatter,
                "cannot {action} step {step_id} while it is {from}"
            ),
            Self::InvalidRunTransition {
                run_id,
                from,
                action,
            } => write!(formatter, "cannot {action} run {run_id} while it is {from}"),
            Self::InvalidAttemptOrdinal { ordinal } => {
                write!(formatter, "attempt ordinal {ordinal} is out of sequence")
            }
            Self::InvalidAttemptTime { ordinal } => write!(
                formatter,
                "attempt {ordinal} cannot finish before it started"
            ),
            Self::AttemptAlreadyFinished { ordinal } => {
                write!(formatter, "attempt {ordinal} is already finished")
            }
            Self::NoAttempt { step_id } => {
                write!(formatter, "step {step_id} has no attempt to finish")
            }
            Self::RetryBudgetExhausted {
                step_id,
                max_attempts,
            } => write!(
                formatter,
                "step {step_id} already used its {max_attempts} allowed attempts"
            ),
            Self::IdentifierExhausted { entity } => {
                write!(formatter, "cannot allocate another {entity} identifier")
            }
        }
    }
}

impl Error for RoutineError {}
