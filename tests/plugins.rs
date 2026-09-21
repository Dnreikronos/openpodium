use std::fs;
use std::path::Path;
use std::time::Duration;

use openpodium::plugins::{
    AdapterRequest, Capability, DiagnosticKind, PermissionGrant, PluginCatalog, PluginError,
    PluginSession, PluginState,
};
use tempfile::TempDir;

const SAMPLE_MANIFEST: &str = include_str!("../examples/sample-plugin/plugin.json");
const SAMPLE_BINARY: &str = env!("CARGO_BIN_EXE_openpodium-sample-plugin");

#[test]
fn external_plugin_adds_a_permission_gated_provider_and_command() {
    let (_directory, mut catalog) = installed_sample();
    assert_eq!(catalog.active_commands().count(), 0);
    assert_eq!(catalog.active_providers().count(), 0);

    catalog
        .enable(
            "dev.openpodium.sample",
            PermissionGrant::new([Capability::Commands]),
        )
        .unwrap();
    let (record, command) = catalog.active_commands().next().unwrap();
    assert_eq!(
        record.qualified_id(&command.id),
        "dev.openpodium.sample/say-hello"
    );
    assert_eq!(catalog.active_providers().count(), 0);

    let mut session = PluginSession::start(record).unwrap();
    let outcome = session
        .invoke_command("say-hello", serde_json::json!({ "name": "Ada" }))
        .unwrap();
    assert_eq!(outcome.value["message"], "Hello, Ada!");
    assert_eq!(
        session
            .prepare_adapter(
                "echo",
                AdapterRequest {
                    working_directory: ".".to_owned(),
                    model: None,
                    prompt: None,
                },
            )
            .unwrap_err(),
        PluginError::PermissionDenied(Capability::Adapters)
    );
    session.shutdown();

    catalog.disable("dev.openpodium.sample").unwrap();
    assert_eq!(
        catalog.plugin("dev.openpodium.sample").unwrap().state(),
        PluginState::Disabled
    );
}

#[test]
fn adapter_plans_are_data_and_refresh_applies_valid_updates() {
    let (directory, mut catalog) = installed_sample();
    catalog
        .enable(
            "dev.openpodium.sample",
            PermissionGrant::new([Capability::Adapters, Capability::Commands]),
        )
        .unwrap();
    let record = catalog.plugin("dev.openpodium.sample").unwrap();
    let mut session = PluginSession::start(record).unwrap();
    let plan = session
        .prepare_adapter(
            "echo",
            AdapterRequest {
                working_directory: "/workspace".to_owned(),
                model: Some("echo-1".to_owned()),
                prompt: Some("repeat me".to_owned()),
            },
        )
        .unwrap();
    assert_eq!(plan.program, "sample-agent");
    assert_eq!(plan.arguments, ["repeat me"]);
    assert_eq!(plan.environment["OPENPODIUM_SAMPLE_MODEL"], "echo-1");
    session.shutdown();

    write_manifest(directory.path(), &SAMPLE_MANIFEST.replace("1.0.0", "1.1.0"));
    catalog.refresh();
    let updated = catalog.plugin("dev.openpodium.sample").unwrap();
    assert_eq!(updated.manifest().version, "1.1.0");
    assert_eq!(updated.state(), PluginState::Enabled);
    assert!(
        catalog
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind() == DiagnosticKind::Update)
    );
}

#[test]
fn a_crash_or_invalid_protocol_quarantines_only_that_session() {
    let (_directory, mut catalog) = installed_sample();
    catalog
        .enable(
            "dev.openpodium.sample",
            PermissionGrant::new([Capability::Commands]),
        )
        .unwrap();
    let record = catalog.plugin("dev.openpodium.sample").unwrap();

    let mut crashed = PluginSession::start(record).unwrap();
    assert_eq!(
        crashed
            .invoke_command("say-hello", serde_json::json!({ "crash": true }))
            .unwrap_err(),
        PluginError::Exited
    );
    assert!(crashed.quarantined());

    let mut malformed = PluginSession::start(record).unwrap();
    assert!(matches!(
        malformed.invoke_command("say-hello", serde_json::json!({ "malformed": true })),
        Err(PluginError::Protocol(_))
    ));
    assert!(malformed.quarantined());

    let mut escalation = PluginSession::start(record).unwrap();
    assert!(matches!(
        escalation.invoke_command("say-hello", serde_json::json!({ "escalate": true })),
        Err(PluginError::Protocol(_))
    ));
    assert!(escalation.quarantined());

    let mut timed_out =
        PluginSession::start_with_timeout(record, Duration::from_millis(50)).unwrap();
    assert_eq!(
        timed_out
            .invoke_command("say-hello", serde_json::json!({ "hang": true }))
            .unwrap_err(),
        PluginError::TimedOut
    );
    assert!(timed_out.quarantined());

    let mut recovered = PluginSession::start(record).unwrap();
    assert_eq!(
        recovered
            .invoke_command(
                "say-hello",
                serde_json::json!({ "text": "x".repeat(1024 * 1024) }),
            )
            .unwrap_err(),
        PluginError::RequestTooLarge
    );
    assert!(!recovered.quarantined());
    assert!(
        recovered
            .invoke_command("say-hello", serde_json::json!({}))
            .is_ok()
    );
    assert_eq!(
        catalog.plugin("dev.openpodium.sample").unwrap().state(),
        PluginState::Enabled
    );
}

#[test]
fn incompatible_and_invalid_updates_are_diagnostic_and_non_destructive() {
    let (directory, mut catalog) = installed_sample();
    let manifest_path = directory.path().join("sample").join("plugin.json");
    fs::write(&manifest_path, "{not valid JSON").unwrap();
    catalog.refresh();
    assert_eq!(
        catalog
            .plugin("dev.openpodium.sample")
            .unwrap()
            .manifest()
            .version,
        "1.0.0"
    );
    assert!(
        catalog
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind() == DiagnosticKind::Manifest)
    );

    write_manifest(
        directory.path(),
        &SAMPLE_MANIFEST.replace(
            "\"minimum\": \"1.0\",\n    \"maximum\": \"1.0\"",
            "\"minimum\": \"2.0\",\n    \"maximum\": \"2.0\"",
        ),
    );
    catalog.refresh();
    assert_eq!(
        catalog.plugin("dev.openpodium.sample").unwrap().state(),
        PluginState::Incompatible
    );
    assert!(
        catalog
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind() == DiagnosticKind::Compatibility)
    );
}

fn installed_sample() -> (TempDir, PluginCatalog) {
    let directory = tempfile::tempdir().unwrap();
    let plugin_directory = directory.path().join("sample");
    fs::create_dir(&plugin_directory).unwrap();
    let binary_name = Path::new(SAMPLE_BINARY).file_name().unwrap();
    fs::copy(SAMPLE_BINARY, plugin_directory.join(binary_name)).unwrap();
    write_manifest(directory.path(), SAMPLE_MANIFEST);
    let catalog = PluginCatalog::discover([directory.path().to_path_buf()]);
    assert!(catalog.diagnostics().is_empty());
    (directory, catalog)
}

fn write_manifest(root: &Path, manifest: &str) {
    let binary_name = Path::new(SAMPLE_BINARY)
        .file_name()
        .unwrap()
        .to_string_lossy();
    let manifest = manifest.replace("openpodium-sample-plugin", &binary_name);
    fs::write(root.join("sample").join("plugin.json"), manifest).unwrap();
}
