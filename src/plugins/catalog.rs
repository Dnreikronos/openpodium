use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

use super::manifest::{
    Capability, CommandContribution, HOST_SDK_VERSION, PluginManifest, ProviderContribution,
    SdkVersion,
};

const MANIFEST_FILE: &str = "plugin.json";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionGrant {
    capabilities: BTreeSet<Capability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionImpact {
    Observe,
    Change,
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionReviewItem {
    capability: Capability,
    impact: PermissionImpact,
    summary: &'static str,
    granted: bool,
}

impl PermissionReviewItem {
    pub const fn capability(&self) -> Capability {
        self.capability
    }

    pub const fn impact(&self) -> PermissionImpact {
        self.impact
    }

    pub const fn summary(&self) -> &'static str {
        self.summary
    }

    pub const fn granted(&self) -> bool {
        self.granted
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionReview {
    plugin_id: String,
    plugin_name: String,
    plugin_version: String,
    permissions: Vec<PermissionReviewItem>,
}

impl PermissionReview {
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn plugin_name(&self) -> &str {
        &self.plugin_name
    }

    pub fn plugin_version(&self) -> &str {
        &self.plugin_version
    }

    pub fn permissions(&self) -> &[PermissionReviewItem] {
        &self.permissions
    }

    pub const fn native_access_notice(&self) -> &'static str {
        "Native plugins run as local child processes with the user's filesystem and network access; capability grants only restrict OpenPodium APIs."
    }
}

impl PermissionGrant {
    pub fn new(capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn contains(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }

    pub fn capabilities(&self) -> impl Iterator<Item = Capability> + '_ {
        self.capabilities.iter().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    Disabled,
    Enabled,
    Incompatible,
}

#[derive(Debug, Clone)]
pub struct PluginRecord {
    manifest: PluginManifest,
    directory: PathBuf,
    executable: PathBuf,
    negotiated_sdk: Option<SdkVersion>,
    state: PluginState,
    granted: PermissionGrant,
}

impl PluginRecord {
    pub const fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub const fn negotiated_sdk(&self) -> Option<SdkVersion> {
        self.negotiated_sdk
    }

    pub const fn state(&self) -> PluginState {
        self.state
    }

    pub const fn granted(&self) -> &PermissionGrant {
        &self.granted
    }

    pub fn qualified_id(&self, local_id: &str) -> String {
        format!("{}/{local_id}", self.manifest.id)
    }

    pub fn permission_review(&self) -> PermissionReview {
        let permissions = self
            .manifest
            .capabilities
            .iter()
            .copied()
            .map(|capability| PermissionReviewItem {
                capability,
                impact: permission_impact(capability),
                summary: permission_summary(capability),
                granted: self.granted.contains(capability),
            })
            .collect();
        PermissionReview {
            plugin_id: self.manifest.id.clone(),
            plugin_name: self.manifest.name.clone(),
            plugin_version: self.manifest.version.clone(),
            permissions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    Discovery,
    Manifest,
    Compatibility,
    Duplicate,
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDiagnostic {
    kind: DiagnosticKind,
    path: PathBuf,
    plugin_id: Option<String>,
    message: String,
}

impl PluginDiagnostic {
    pub const fn kind(&self) -> DiagnosticKind {
        self.kind
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn plugin_id(&self) -> Option<&str> {
        self.plugin_id.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone)]
pub struct PluginCatalog {
    roots: Vec<PathBuf>,
    records: BTreeMap<String, PluginRecord>,
    diagnostics: Vec<PluginDiagnostic>,
}

impl PluginCatalog {
    pub fn discover(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut catalog = Self {
            roots: roots.into_iter().collect(),
            records: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        catalog.refresh();
        catalog
    }

    pub fn refresh(&mut self) {
        let previous = self.records.clone();
        let mut records: BTreeMap<String, PluginRecord> = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let candidates = discover_candidates(&self.roots, &mut diagnostics);

        for manifest_path in candidates {
            match load_record(&manifest_path) {
                Ok(mut record) => {
                    if let Some(existing) = records.get(&record.manifest.id) {
                        diagnostics.push(diagnostic(
                            DiagnosticKind::Duplicate,
                            manifest_path,
                            Some(record.manifest.id.clone()),
                            format!(
                                "duplicate plugin id; keeping {}",
                                existing.directory.display()
                            ),
                        ));
                        continue;
                    }
                    if let Some(existing) = previous.get(&record.manifest.id)
                        && existing.directory == record.directory
                    {
                        record.granted =
                            PermissionGrant::new(existing.granted.capabilities().filter(
                                |capability| record.manifest.capabilities.contains(capability),
                            ));
                        if existing.state == PluginState::Enabled && record.negotiated_sdk.is_some()
                        {
                            record.state = PluginState::Enabled;
                        }
                        if existing.manifest.version != record.manifest.version {
                            diagnostics.push(diagnostic(
                                DiagnosticKind::Update,
                                manifest_path.clone(),
                                Some(record.manifest.id.clone()),
                                format!(
                                    "updated from {} to {}",
                                    existing.manifest.version, record.manifest.version
                                ),
                            ));
                        }
                    }
                    if record.negotiated_sdk.is_none() {
                        diagnostics.push(diagnostic(
                            DiagnosticKind::Compatibility,
                            manifest_path,
                            Some(record.manifest.id.clone()),
                            record
                                .manifest
                                .negotiate(HOST_SDK_VERSION)
                                .expect_err("an incompatible record must fail negotiation")
                                .to_string(),
                        ));
                    }
                    records.insert(record.manifest.id.clone(), record);
                }
                Err((kind, message)) => {
                    let resolved_manifest =
                        fs::canonicalize(&manifest_path).unwrap_or_else(|_| manifest_path.clone());
                    let retained = previous
                        .values()
                        .find(|record| record.directory.join(MANIFEST_FILE) == resolved_manifest);
                    if let Some(record) = retained {
                        records.insert(record.manifest.id.clone(), record.clone());
                    }
                    diagnostics.push(diagnostic(kind, manifest_path, None, message));
                }
            }
        }

        self.records = records;
        self.diagnostics = diagnostics;
    }

    pub fn plugins(&self) -> impl Iterator<Item = &PluginRecord> {
        self.records.values()
    }

    pub fn plugin(&self, plugin_id: &str) -> Option<&PluginRecord> {
        self.records.get(plugin_id)
    }

    pub fn diagnostics(&self) -> &[PluginDiagnostic] {
        &self.diagnostics
    }

    pub fn permission_review(&self, plugin_id: &str) -> Result<PermissionReview, CatalogError> {
        self.plugin(plugin_id)
            .map(PluginRecord::permission_review)
            .ok_or_else(|| CatalogError::NotFound(plugin_id.to_owned()))
    }

    pub fn enable(&mut self, plugin_id: &str, grant: PermissionGrant) -> Result<(), CatalogError> {
        let record = self
            .records
            .get_mut(plugin_id)
            .ok_or_else(|| CatalogError::NotFound(plugin_id.to_owned()))?;
        if record.negotiated_sdk.is_none() {
            return Err(CatalogError::Incompatible(plugin_id.to_owned()));
        }
        if let Some(capability) = grant
            .capabilities()
            .find(|capability| !record.manifest.capabilities.contains(capability))
        {
            return Err(CatalogError::UndeclaredPermission {
                plugin_id: plugin_id.to_owned(),
                capability,
            });
        }
        record.granted = grant;
        record.state = PluginState::Enabled;
        Ok(())
    }

    pub fn disable(&mut self, plugin_id: &str) -> Result<(), CatalogError> {
        let record = self
            .records
            .get_mut(plugin_id)
            .ok_or_else(|| CatalogError::NotFound(plugin_id.to_owned()))?;
        record.granted = PermissionGrant::default();
        record.state = if record.negotiated_sdk.is_some() {
            PluginState::Disabled
        } else {
            PluginState::Incompatible
        };
        Ok(())
    }

    pub fn active_plugins(&self) -> impl Iterator<Item = &PluginRecord> {
        self.records
            .values()
            .filter(|record| record.state == PluginState::Enabled)
    }

    pub fn active_providers(&self) -> impl Iterator<Item = (&PluginRecord, &ProviderContribution)> {
        self.active_plugins()
            .filter(|record| record.granted.contains(Capability::Adapters))
            .flat_map(|record| {
                record
                    .manifest
                    .contributions
                    .providers
                    .iter()
                    .map(move |provider| (record, provider))
            })
    }

    pub fn active_commands(&self) -> impl Iterator<Item = (&PluginRecord, &CommandContribution)> {
        self.active_plugins()
            .filter(|record| record.granted.contains(Capability::Commands))
            .flat_map(|record| {
                record
                    .manifest
                    .contributions
                    .commands
                    .iter()
                    .map(move |command| (record, command))
            })
    }
}

fn discover_candidates(roots: &[PathBuf], diagnostics: &mut Vec<PluginDiagnostic>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for root in roots {
        let direct = root.join(MANIFEST_FILE);
        add_manifest_candidate(&direct, &mut candidates, diagnostics);
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) => {
                diagnostics.push(diagnostic(
                    DiagnosticKind::Discovery,
                    root.clone(),
                    None,
                    format!("could not scan plugin directory: {error}"),
                ));
                continue;
            }
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
                diagnostics.push(diagnostic(
                    DiagnosticKind::Discovery,
                    entry.path(),
                    None,
                    "plugin directories cannot be symbolic links",
                ));
                continue;
            }
            let manifest = entry.path().join(MANIFEST_FILE);
            add_manifest_candidate(&manifest, &mut candidates, diagnostics);
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn add_manifest_candidate(
    path: &Path,
    candidates: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<PluginDiagnostic>,
) {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => diagnostics.push(diagnostic(
            DiagnosticKind::Discovery,
            path.to_owned(),
            None,
            "plugin manifests cannot be symbolic links",
        )),
        Ok(metadata) if metadata.is_file() => candidates.push(path.to_owned()),
        Ok(_) | Err(_) => {}
    }
}

fn load_record(path: &Path) -> Result<PluginRecord, (DiagnosticKind, String)> {
    let json = fs::read_to_string(path).map_err(|error| {
        (
            DiagnosticKind::Discovery,
            format!("could not read plugin manifest: {error}"),
        )
    })?;
    let manifest = PluginManifest::from_json(&json)
        .map_err(|error| (DiagnosticKind::Manifest, error.to_string()))?;
    let directory = path
        .parent()
        .expect("a manifest candidate always has a parent")
        .to_path_buf();
    let directory = fs::canonicalize(&directory).map_err(|error| {
        (
            DiagnosticKind::Discovery,
            format!("could not resolve plugin directory: {error}"),
        )
    })?;
    let unresolved_executable = directory.join(&manifest.executable);
    let executable = fs::canonicalize(&unresolved_executable).map_err(|_| {
        (
            DiagnosticKind::Discovery,
            format!(
                "plugin executable does not exist: {}",
                unresolved_executable.display()
            ),
        )
    })?;
    if !executable.starts_with(&directory) || !executable.is_file() {
        return Err((
            DiagnosticKind::Discovery,
            "plugin executable must resolve inside its plugin directory".to_owned(),
        ));
    }
    let negotiated_sdk = manifest.negotiate(HOST_SDK_VERSION).ok();
    let state = if negotiated_sdk.is_some() {
        PluginState::Disabled
    } else {
        PluginState::Incompatible
    };
    Ok(PluginRecord {
        manifest,
        directory,
        executable,
        negotiated_sdk,
        state,
        granted: PermissionGrant::default(),
    })
}

fn permission_impact(capability: Capability) -> PermissionImpact {
    match capability {
        Capability::Events | Capability::SettingsRead => PermissionImpact::Observe,
        Capability::SettingsWrite | Capability::Ui => PermissionImpact::Change,
        Capability::Adapters | Capability::Commands => PermissionImpact::Execute,
    }
}

fn permission_summary(capability: Capability) -> &'static str {
    match capability {
        Capability::Adapters => "Prepare executable agent launch plans",
        Capability::Commands => "Receive user-invoked plugin commands",
        Capability::Events => "Receive explicitly forwarded host events",
        Capability::SettingsRead => "Read the plugin's namespaced settings",
        Capability::SettingsWrite => "Propose changes to the plugin's namespaced settings",
        Capability::Ui => "Show host-rendered panels and settings fields",
    }
}

fn diagnostic(
    kind: DiagnosticKind,
    path: PathBuf,
    plugin_id: Option<String>,
    message: impl Into<String>,
) -> PluginDiagnostic {
    PluginDiagnostic {
        kind,
        path,
        plugin_id,
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    NotFound(String),
    Incompatible(String),
    UndeclaredPermission {
        plugin_id: String,
        capability: Capability,
    },
}

impl Display for CatalogError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(plugin_id) => write!(formatter, "plugin `{plugin_id}` was not found"),
            Self::Incompatible(plugin_id) => {
                write!(
                    formatter,
                    "plugin `{plugin_id}` is incompatible with this SDK"
                )
            }
            Self::UndeclaredPermission {
                plugin_id,
                capability,
            } => write!(
                formatter,
                "plugin `{plugin_id}` did not declare the `{capability}` capability"
            ),
        }
    }
}

impl Error for CatalogError {}
