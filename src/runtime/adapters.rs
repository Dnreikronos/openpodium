use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display, Formatter};
use std::path::Path;

use crate::domain::{AgentProgram, CommandPreset, Role};

use super::executables::executable_exists;
use super::{ProcessSpec, TerminalSize};

pub const ROLE_INSTRUCTIONS_ENV: &str = "OPENPODIUM_ROLE_INSTRUCTIONS";
const OPENCODE_AGENT_NAME: &str = "openpodium";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCapability {
    executable: OsString,
    installed: bool,
    setup_guidance: Option<String>,
}

impl AgentCapability {
    pub fn executable(&self) -> &OsStr {
        &self.executable
    }

    pub const fn installed(&self) -> bool {
        self.installed
    }

    pub fn setup_guidance(&self) -> Option<&str> {
        self.setup_guidance.as_deref()
    }
}

pub fn check_agent_capability(
    program: AgentProgram,
    preset: Option<&CommandPreset>,
) -> Result<AgentCapability, AgentAdapterError> {
    let executable = resolve_executable(program, preset)?;
    let installed = executable_exists(&executable);
    Ok(AgentCapability {
        setup_guidance: (!installed).then(|| setup_guidance(program, &executable)),
        executable,
        installed,
    })
}

pub fn prepare_agent_process(
    program: AgentProgram,
    preset: Option<&CommandPreset>,
    role: Option<&Role>,
    working_directory: &Path,
    terminal_size: TerminalSize,
) -> Result<ProcessSpec, AgentAdapterError> {
    let mut spec = match program {
        AgentProgram::Codex => ProcessSpec::new("codex", working_directory),
        AgentProgram::Claude => ProcessSpec::new("claude", working_directory),
        AgentProgram::OpenCode => ProcessSpec::new("opencode", working_directory),
        AgentProgram::Shell => shell_spec(working_directory),
        AgentProgram::Custom(preset_id) => {
            let preset = preset.filter(|preset| preset.id() == preset_id).ok_or(
                AgentAdapterError::MissingPreset {
                    program_preset_id: preset_id,
                },
            )?;
            ProcessSpec::new(preset.executable(), working_directory)
                .args(preset.arguments().iter().map(OsString::from))
        }
    };

    if let Some(role) = role {
        let instructions = role.instructions().as_str();
        spec = match program {
            AgentProgram::Codex => spec.args([
                OsString::from("-c"),
                OsString::from(format!(
                    "developer_instructions={}",
                    serde_json::to_string(instructions)
                        .expect("serializing a string to JSON cannot fail")
                )),
            ]),
            AgentProgram::Claude => spec.args([
                OsString::from("--append-system-prompt"),
                OsString::from(instructions),
            ]),
            AgentProgram::OpenCode => spec
                .args([
                    OsString::from("--agent"),
                    OsString::from(OPENCODE_AGENT_NAME),
                ])
                .env(
                    "OPENCODE_CONFIG_CONTENT",
                    serde_json::json!({
                        "agents": {
                            OPENCODE_AGENT_NAME: {
                                "mode": "primary",
                                "system": instructions
                            }
                        }
                    })
                    .to_string(),
                ),
            AgentProgram::Shell | AgentProgram::Custom(_) => spec,
        };
        spec = spec.env(ROLE_INSTRUCTIONS_ENV, instructions);
    }

    Ok(spec
        .with_terminal_size(terminal_size)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("TERM_PROGRAM", "OpenPodium"))
}

fn resolve_executable(
    program: AgentProgram,
    preset: Option<&CommandPreset>,
) -> Result<OsString, AgentAdapterError> {
    match program {
        AgentProgram::Shell => Ok(shell_program()),
        AgentProgram::Custom(preset_id) => preset
            .filter(|preset| preset.id() == preset_id)
            .map(|preset| OsString::from(preset.executable()))
            .ok_or(AgentAdapterError::MissingPreset {
                program_preset_id: preset_id,
            }),
        _ => {
            Ok(OsString::from(program.executable().expect(
                "non-shell built-in adapters define an executable",
            )))
        }
    }
}

fn setup_guidance(program: AgentProgram, executable: &OsStr) -> String {
    match program {
        AgentProgram::Codex => {
            "Codex is not installed. Install it with `npm install -g @openai/codex`.".to_owned()
        }
        AgentProgram::Claude => {
            "Claude Code is not installed. Follow https://code.claude.com/docs/en/setup.".to_owned()
        }
        AgentProgram::OpenCode => {
            "OpenCode is not installed. Follow https://opencode.ai/docs.".to_owned()
        }
        AgentProgram::Shell => {
            "The configured login shell is unavailable. Check SHELL or COMSPEC.".to_owned()
        }
        AgentProgram::Custom(_) => format!(
            "The preset executable {executable:?} is unavailable. Install it or update the preset."
        ),
    }
}

#[cfg(unix)]
fn shell_program() -> OsString {
    env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into())
}

#[cfg(windows)]
fn shell_program() -> OsString {
    env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into())
}

#[cfg(unix)]
fn shell_spec(working_directory: &Path) -> ProcessSpec {
    ProcessSpec::new(shell_program(), working_directory).arg("-l")
}

#[cfg(windows)]
fn shell_spec(working_directory: &Path) -> ProcessSpec {
    ProcessSpec::new(shell_program(), working_directory)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAdapterError {
    MissingPreset {
        program_preset_id: crate::domain::CommandPresetId,
    },
}

impl Display for AgentAdapterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPreset { program_preset_id } => write!(
                formatter,
                "custom agent references missing command preset {program_preset_id}"
            ),
        }
    }
}

impl Error for AgentAdapterError {}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use crate::domain::{CommandPresetId, Content, Name, RoleColor, RoleIcon, RoleId};

    use super::*;

    #[test]
    fn every_builtin_adapter_has_a_previewable_argument_array_and_environment() {
        let role = test_role("Review `$(touch /tmp/unsafe)` and preserve $HOME.");
        let size = TerminalSize::new(30, 100).unwrap();

        let codex = prepare_agent_process(
            AgentProgram::Codex,
            None,
            Some(&role),
            Path::new("/workspace"),
            size,
        )
        .unwrap();
        assert_eq!(codex.program(), OsStr::new("codex"));
        assert_eq!(codex.arguments()[0], OsString::from("-c"));
        assert!(
            codex.arguments()[1]
                .to_string_lossy()
                .starts_with("developer_instructions=\"")
        );

        let claude = prepare_agent_process(
            AgentProgram::Claude,
            None,
            Some(&role),
            Path::new("/workspace"),
            size,
        )
        .unwrap();
        assert_eq!(
            claude.arguments(),
            [
                OsString::from("--append-system-prompt"),
                OsString::from(role.instructions().as_str())
            ]
        );

        let opencode = prepare_agent_process(
            AgentProgram::OpenCode,
            None,
            Some(&role),
            Path::new("/workspace"),
            size,
        )
        .unwrap();
        assert_eq!(
            opencode.arguments(),
            [OsString::from("--agent"), OsString::from("openpodium")]
        );
        let config = environment_value(&opencode, "OPENCODE_CONFIG_CONTENT");
        let config: serde_json::Value = serde_json::from_str(config).unwrap();
        assert_eq!(
            config["agents"]["openpodium"]["system"],
            role.instructions().as_str()
        );

        let shell = prepare_agent_process(
            AgentProgram::Shell,
            None,
            Some(&role),
            Path::new("/workspace"),
            size,
        )
        .unwrap();
        assert_eq!(
            environment_value(&shell, ROLE_INSTRUCTIONS_ENV),
            role.instructions().as_str()
        );
        for spec in [&codex, &claude, &opencode, &shell] {
            assert_eq!(environment_value(spec, "TERM"), "xterm-256color");
            assert_eq!(spec.terminal_size(), size);
        }
    }

    #[test]
    fn custom_presets_remain_an_executable_and_distinct_arguments() {
        let preset = CommandPreset::new(
            CommandPresetId::new(7),
            Name::new("Custom").unwrap(),
            "agent-cli",
            vec!["--prompt".to_owned(), "$(touch /tmp/unsafe)".to_owned()],
        )
        .unwrap();
        let role = test_role("Keep this as one environment value\nwith two lines.");

        let spec = prepare_agent_process(
            AgentProgram::Custom(preset.id()),
            Some(&preset),
            Some(&role),
            Path::new("/workspace"),
            TerminalSize::default(),
        )
        .unwrap();

        assert_eq!(spec.program(), OsStr::new("agent-cli"));
        assert_eq!(
            spec.arguments(),
            ["--prompt", "$(touch /tmp/unsafe)"].map(OsString::from)
        );
        assert_eq!(
            environment_value(&spec, ROLE_INSTRUCTIONS_ENV),
            role.instructions().as_str()
        );
    }

    #[test]
    fn missing_executables_return_setup_guidance_without_spawning() {
        let preset = CommandPreset::new(
            CommandPresetId::new(1),
            Name::new("Missing").unwrap(),
            "/openpodium/does/not/exist",
            Vec::new(),
        )
        .unwrap();

        let capability =
            check_agent_capability(AgentProgram::Custom(preset.id()), Some(&preset)).unwrap();

        assert!(!capability.installed());
        assert!(
            capability
                .setup_guidance()
                .unwrap()
                .contains("update the preset")
        );
    }

    fn environment_value<'a>(spec: &'a ProcessSpec, name: &str) -> &'a str {
        spec.environment()
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| value.to_str())
            .unwrap()
    }

    fn test_role(instructions: &str) -> Role {
        Role::with_appearance(
            RoleId::new(1),
            Name::new("Reviewer").unwrap(),
            RoleColor::new("#8B5CF6").unwrap(),
            RoleIcon::new("review").unwrap(),
            Content::new(instructions).unwrap(),
        )
    }
}
