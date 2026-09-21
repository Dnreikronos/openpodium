use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const HOST_SDK_VERSION: SdkVersion = SdkVersion { major: 1, minor: 0 };

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SdkVersion {
    major: u16,
    minor: u16,
}

impl SdkVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    pub const fn major(self) -> u16 {
        self.major
    }

    pub const fn minor(self) -> u16 {
        self.minor
    }
}

impl Display for SdkVersion {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for SdkVersion {
    type Err = ManifestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut parts = value.split('.');
        let major = parse_version_part(parts.next(), "major", value)?;
        let minor = parse_version_part(parts.next(), "minor", value)?;
        if parts.next().is_some() {
            return Err(ManifestError::new(format!(
                "SDK version `{value}` must use major.minor"
            )));
        }
        Ok(Self { major, minor })
    }
}

impl Serialize for SdkVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SdkVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

fn parse_version_part(
    part: Option<&str>,
    name: &str,
    complete: &str,
) -> Result<u16, ManifestError> {
    part.filter(|part| !part.is_empty())
        .ok_or_else(|| ManifestError::new(format!("SDK version `{complete}` has no {name}")))?
        .parse()
        .map_err(|_| ManifestError::new(format!("SDK version `{complete}` has an invalid {name}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkRange {
    pub minimum: SdkVersion,
    pub maximum: SdkVersion,
}

impl SdkRange {
    pub fn negotiate(self, host: SdkVersion) -> Result<SdkVersion, CompatibilityError> {
        if self.minimum > self.maximum {
            return Err(CompatibilityError::InvalidRange {
                minimum: self.minimum,
                maximum: self.maximum,
            });
        }
        if host < self.minimum || host > self.maximum {
            return Err(CompatibilityError::Unsupported {
                host,
                minimum: self.minimum,
                maximum: self.maximum,
            });
        }
        Ok(host)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "adapters")]
    Adapters,
    #[serde(rename = "commands")]
    Commands,
    #[serde(rename = "events")]
    Events,
    #[serde(rename = "settings.read")]
    SettingsRead,
    #[serde(rename = "settings.write")]
    SettingsWrite,
    #[serde(rename = "ui")]
    Ui,
}

impl Capability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Adapters => "adapters",
            Self::Commands => "commands",
            Self::Events => "events",
            Self::SettingsRead => "settings.read",
            Self::SettingsWrite => "settings.write",
            Self::Ui => "ui",
        }
    }
}

impl Display for Capability {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub sdk: SdkRange,
    pub executable: PathBuf,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub contributions: Contributions,
}

impl PluginManifest {
    pub fn from_json(json: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(json)
            .map_err(|error| ManifestError::new(format!("invalid plugin JSON: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        validate_plugin_id(&self.id)?;
        require_text("plugin name", &self.name)?;
        validate_plugin_version(&self.version)?;
        self.sdk
            .negotiate(self.sdk.minimum)
            .map_err(|error| ManifestError::new(error.to_string()))?;
        validate_executable(&self.executable)?;

        validate_contributions(
            "provider",
            self.contributions.providers.iter().map(|item| &item.id),
        )?;
        validate_contributions(
            "command",
            self.contributions.commands.iter().map(|item| &item.id),
        )?;
        validate_contributions(
            "setting",
            self.contributions.settings.iter().map(|item| &item.key),
        )?;
        validate_contributions(
            "panel",
            self.contributions.panels.iter().map(|item| &item.id),
        )?;

        for provider in &self.contributions.providers {
            require_capability(self, Capability::Adapters, "provider")?;
            require_text("provider label", &provider.label)?;
            if provider.models.is_empty()
                || provider.models.iter().any(|model| model.trim().is_empty())
            {
                return Err(ManifestError::new(format!(
                    "provider `{}` must declare non-empty model identifiers",
                    provider.id
                )));
            }
        }
        for command in &self.contributions.commands {
            require_capability(self, Capability::Commands, "command")?;
            require_text("command label", &command.label)?;
        }
        if !self.contributions.events.is_empty() {
            require_capability(self, Capability::Events, "event subscription")?;
            for event in &self.contributions.events {
                validate_local_id("event", event)?;
            }
        }
        for setting in &self.contributions.settings {
            if !self.capabilities.contains(&Capability::SettingsRead)
                && !self.capabilities.contains(&Capability::SettingsWrite)
            {
                return Err(ManifestError::new(format!(
                    "setting `{}` requires settings.read or settings.write",
                    setting.key
                )));
            }
            require_text("setting label", &setting.label)?;
        }
        for panel in &self.contributions.panels {
            require_capability(self, Capability::Ui, "panel")?;
            require_text("panel label", &panel.label)?;
        }
        Ok(())
    }

    pub fn negotiate(&self, host: SdkVersion) -> Result<SdkVersion, CompatibilityError> {
        self.sdk.negotiate(host)
    }
}

fn require_capability(
    manifest: &PluginManifest,
    capability: Capability,
    contribution: &str,
) -> Result<(), ManifestError> {
    if manifest.capabilities.contains(&capability) {
        Ok(())
    } else {
        Err(ManifestError::new(format!(
            "{contribution} contributions require the `{capability}` capability"
        )))
    }
}

fn validate_plugin_id(value: &str) -> Result<(), ManifestError> {
    if value.split('.').count() < 2 || !valid_identifier(value, true) {
        return Err(ManifestError::new(format!(
            "plugin id `{value}` must be a lowercase reverse-DNS identifier"
        )));
    }
    Ok(())
}

fn validate_plugin_version(value: &str) -> Result<(), ManifestError> {
    let core = value.split_once('-').map_or(value, |(core, _)| core);
    let parts: Vec<_> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || part.parse::<u64>().is_err())
    {
        return Err(ManifestError::new(format!(
            "plugin version `{value}` must use semantic major.minor.patch"
        )));
    }
    Ok(())
}

fn validate_executable(value: &Path) -> Result<(), ManifestError> {
    if value.as_os_str().is_empty()
        || value.is_absolute()
        || value
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ManifestError::new(
            "plugin executable must be a relative path without parent traversal",
        ));
    }
    Ok(())
}

fn validate_contributions<'a>(
    kind: &str,
    identifiers: impl Iterator<Item = &'a String>,
) -> Result<(), ManifestError> {
    let mut seen = BTreeSet::new();
    for identifier in identifiers {
        validate_local_id(kind, identifier)?;
        if !seen.insert(identifier) {
            return Err(ManifestError::new(format!(
                "duplicate {kind} contribution `{identifier}`"
            )));
        }
    }
    Ok(())
}

fn validate_local_id(kind: &str, value: &str) -> Result<(), ManifestError> {
    if !valid_identifier(value, false) {
        return Err(ManifestError::new(format!(
            "{kind} identifier `{value}` contains unsupported characters"
        )));
    }
    Ok(())
}

fn valid_identifier(value: &str, require_dot: bool) -> bool {
    !value.is_empty()
        && (!require_dot || value.contains('.'))
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn require_text(field: &str, value: &str) -> Result<(), ManifestError> {
    if value.trim().is_empty() {
        Err(ManifestError::new(format!("{field} cannot be empty")))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributions {
    #[serde(default)]
    pub providers: Vec<ProviderContribution>,
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub settings: Vec<SettingContribution>,
    #[serde(default)]
    pub panels: Vec<PanelContribution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderContribution {
    pub id: String,
    pub label: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandContribution {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingContribution {
    pub key: String,
    pub label: String,
    pub kind: SettingKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    Boolean,
    Text,
    Secret,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelContribution {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    message: String,
}

impl ManifestError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for ManifestError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ManifestError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityError {
    InvalidRange {
        minimum: SdkVersion,
        maximum: SdkVersion,
    },
    Unsupported {
        host: SdkVersion,
        minimum: SdkVersion,
        maximum: SdkVersion,
    },
}

impl Display for CompatibilityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange { minimum, maximum } => write!(
                formatter,
                "plugin SDK range {minimum}..={maximum} is reversed"
            ),
            Self::Unsupported {
                host,
                minimum,
                maximum,
            } => write!(
                formatter,
                "host SDK {host} is outside plugin range {minimum}..={maximum}"
            ),
        }
    }
}

impl Error for CompatibilityError {}
