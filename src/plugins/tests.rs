use super::*;

fn manifest_json(extra: &str) -> String {
    format!(
        r#"{{
          "id": "dev.openpodium.sample",
          "name": "Sample",
          "version": "1.0.0",
          "sdk": {{ "minimum": "1.0", "maximum": "1.0" }},
          "executable": "sample-plugin",
          "capabilities": ["adapters", "commands"],
          "contributions": {{
            "providers": [{{ "id": "echo", "label": "Echo", "models": ["echo-1"] }}],
            "commands": [{{ "id": "say-hello", "label": "Say hello" }}]
          }}
          {extra}
        }}"#
    )
}

#[test]
fn manifest_negotiates_the_host_sdk_and_validates_capabilities() {
    let manifest = PluginManifest::from_json(&manifest_json("")).unwrap();
    assert_eq!(manifest.negotiate(HOST_SDK_VERSION), Ok(HOST_SDK_VERSION));

    let missing_capability = manifest_json("").replace(
        r#""capabilities": ["adapters", "commands"]"#,
        r#""capabilities": ["commands"]"#,
    );
    assert!(
        PluginManifest::from_json(&missing_capability)
            .unwrap_err()
            .to_string()
            .contains("provider contributions require")
    );
}

#[test]
fn incompatible_sdk_versions_are_rejected() {
    let manifest = PluginManifest::from_json(&manifest_json("").replace(
        r#""minimum": "1.0", "maximum": "1.0""#,
        r#""minimum": "2.0", "maximum": "2.3""#,
    ))
    .unwrap();
    assert!(matches!(
        manifest.negotiate(HOST_SDK_VERSION),
        Err(CompatibilityError::Unsupported { .. })
    ));
}

#[test]
fn paths_and_duplicate_contributions_are_rejected() {
    let traversal = manifest_json("").replace("sample-plugin", "../sample-plugin");
    assert!(PluginManifest::from_json(&traversal).is_err());

    let mut value: serde_json::Value = serde_json::from_str(&manifest_json("")).unwrap();
    let command = value["contributions"]["commands"][0].clone();
    value["contributions"]["commands"]
        .as_array_mut()
        .unwrap()
        .push(command);
    assert!(PluginManifest::from_json(&value.to_string()).is_err());
}
