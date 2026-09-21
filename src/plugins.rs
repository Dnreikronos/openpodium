//! Versioned, capability-gated integration boundary for third-party plugins.

mod catalog;
mod manifest;
mod protocol;

pub use catalog::{
    CatalogError, DiagnosticKind, PermissionGrant, PermissionImpact, PermissionReview,
    PermissionReviewItem, PluginCatalog, PluginDiagnostic, PluginRecord, PluginState,
};
pub use manifest::{
    Capability, CommandContribution, CompatibilityError, Contributions, HOST_SDK_VERSION,
    ManifestError, PanelContribution, PluginManifest, ProviderContribution, SdkRange, SdkVersion,
    SettingContribution, SettingKind,
};
pub use protocol::{
    AdapterPlan, AdapterRequest, CommandOutcome, HostMessage, PluginAction, PluginError,
    PluginMessage, PluginSession,
};

#[cfg(test)]
mod tests;
