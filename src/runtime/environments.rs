use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::domain::{EnvironmentKind, EnvironmentProfile};

use super::{ProcessSpec, RuntimeError, RuntimeOperation};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(6);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(25);
const SSH_MISSING_DIRECTORY_EXIT: i32 = 44;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnvironmentHealth {
    #[default]
    Unknown,
    Checking,
    Healthy,
    MissingExecutable,
    AuthenticationFailed,
    Unreachable,
    InvalidWorkingDirectory,
}

impl EnvironmentHealth {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Checking => "checking",
            Self::Healthy => "healthy",
            Self::MissingExecutable => "missing executable",
            Self::AuthenticationFailed => "authentication failed",
            Self::Unreachable => "unreachable",
            Self::InvalidWorkingDirectory => "invalid working directory",
        }
    }
}

pub fn prepare_environment_process(
    profile: Option<&EnvironmentProfile>,
    command: ProcessSpec,
) -> Result<ProcessSpec, RuntimeError> {
    let Some(profile) = profile else {
        return Ok(command);
    };
    match profile.kind() {
        EnvironmentKind::Ssh(environment) => prepare_ssh(environment, command),
        EnvironmentKind::Container(environment) => Ok(prepare_container(environment, command)),
        EnvironmentKind::Custom(environment) => Ok(prepare_custom(environment, command)),
    }
}

pub fn check_environment(
    profile: Option<&EnvironmentProfile>,
    local_working_directory: &Path,
) -> EnvironmentHealth {
    let Some(profile) = profile else {
        return directory_health(local_working_directory);
    };
    match profile.kind() {
        EnvironmentKind::Ssh(environment) => {
            if !executable_exists(OsStr::new("ssh")) {
                return EnvironmentHealth::MissingExecutable;
            }
            let mut command = Command::new("ssh");
            command.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"]);
            if let Some(port) = environment.port() {
                command.args(["-p", &port.to_string()]);
            }
            command.arg(ssh_destination(environment.user(), environment.host()));
            command.arg(format!(
                "if test -d {}; then exit 0; else exit {SSH_MISSING_DIRECTORY_EXIT}; fi",
                shell_quote(environment.working_directory())
            ));
            match run_probe(command) {
                Ok(output) if output.status.success() => EnvironmentHealth::Healthy,
                Ok(output) if output.status.code() == Some(SSH_MISSING_DIRECTORY_EXIT) => {
                    EnvironmentHealth::InvalidWorkingDirectory
                }
                Ok(output) if authentication_failed(&output.stderr) => {
                    EnvironmentHealth::AuthenticationFailed
                }
                Ok(_) | Err(ProbeError::TimedOut) => EnvironmentHealth::Unreachable,
                Err(ProbeError::Spawn(error)) if error.kind() == io::ErrorKind::NotFound => {
                    EnvironmentHealth::MissingExecutable
                }
                Err(ProbeError::Spawn(_)) => EnvironmentHealth::Unreachable,
            }
        }
        EnvironmentKind::Container(environment) => {
            if !executable_exists(OsStr::new(environment.engine())) {
                return EnvironmentHealth::MissingExecutable;
            }
            let mut inspect = Command::new(environment.engine());
            inspect.args(["inspect", environment.container()]);
            match run_probe(inspect) {
                Ok(output) if output.status.success() => {}
                Ok(_) | Err(ProbeError::TimedOut) => return EnvironmentHealth::Unreachable,
                Err(ProbeError::Spawn(error)) if error.kind() == io::ErrorKind::NotFound => {
                    return EnvironmentHealth::MissingExecutable;
                }
                Err(ProbeError::Spawn(_)) => return EnvironmentHealth::Unreachable,
            }
            let mut directory = Command::new(environment.engine());
            directory.args([
                "exec",
                environment.container(),
                "test",
                "-d",
                environment.working_directory(),
            ]);
            match run_probe(directory) {
                Ok(output) if output.status.success() => EnvironmentHealth::Healthy,
                Ok(_) => EnvironmentHealth::InvalidWorkingDirectory,
                Err(_) => EnvironmentHealth::Unreachable,
            }
        }
        EnvironmentKind::Custom(environment) => {
            if !executable_exists(OsStr::new(environment.executable())) {
                return EnvironmentHealth::MissingExecutable;
            }
            directory_health(Path::new(environment.working_directory().as_str()))
        }
    }
}

fn prepare_ssh(
    environment: &crate::domain::SshEnvironment,
    command: ProcessSpec,
) -> Result<ProcessSpec, RuntimeError> {
    let ProcessSpec {
        program,
        arguments,
        working_directory,
        environment: variables,
        terminal_size,
        output_capacity,
    } = command;
    let program = unicode_value("program", &program)?;
    let arguments = arguments
        .iter()
        .map(|argument| unicode_value("argument", argument))
        .collect::<Result<Vec<_>, _>>()?;
    let variables = variables
        .iter()
        .map(|(name, value)| {
            Ok(format!(
                "{}={}",
                unicode_value("environment name", name)?,
                unicode_value("environment value", value)?
            ))
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;

    let mut remote_tokens = vec![
        "cd".to_owned(),
        "--".to_owned(),
        environment.working_directory().to_owned(),
        "&&".to_owned(),
        "exec".to_owned(),
        "env".to_owned(),
    ];
    remote_tokens.extend(variables);
    remote_tokens.push(program);
    remote_tokens.extend(arguments);
    let remote_command = remote_tokens
        .iter()
        .map(|token| {
            if token == "&&" {
                token.clone()
            } else {
                shell_quote(token)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let mut prepared = ProcessSpec::new("ssh", working_directory).args(["-tt"]);
    if let Some(port) = environment.port() {
        prepared = prepared.args([OsString::from("-p"), OsString::from(port.to_string())]);
    }
    prepared = prepared
        .arg(ssh_destination(environment.user(), environment.host()))
        .arg(remote_command)
        .with_terminal_size(terminal_size)
        .with_output_capacity(output_capacity);
    Ok(prepared)
}

fn prepare_container(
    environment: &crate::domain::ContainerEnvironment,
    command: ProcessSpec,
) -> ProcessSpec {
    let ProcessSpec {
        program,
        arguments,
        working_directory,
        environment: variables,
        terminal_size,
        output_capacity,
    } = command;
    let mut prepared = ProcessSpec::new(environment.engine(), working_directory).args([
        "exec",
        "-it",
        "--workdir",
        environment.working_directory(),
    ]);
    for (name, value) in variables {
        let mut assignment = name;
        assignment.push("=");
        assignment.push(value);
        prepared = prepared.args([OsString::from("--env"), assignment]);
    }
    prepared
        .arg(environment.container())
        .arg(program)
        .args(arguments)
        .with_terminal_size(terminal_size)
        .with_output_capacity(output_capacity)
}

fn prepare_custom(
    environment: &crate::domain::CustomEnvironment,
    command: ProcessSpec,
) -> ProcessSpec {
    let ProcessSpec {
        program,
        arguments,
        environment: variables,
        terminal_size,
        output_capacity,
        ..
    } = command;
    let mut prepared = ProcessSpec::new(
        environment.executable(),
        environment.working_directory().as_str(),
    )
    .args(environment.arguments().iter().map(OsString::from))
    .arg(program)
    .args(arguments)
    .with_terminal_size(terminal_size)
    .with_output_capacity(output_capacity);
    for (name, value) in variables {
        prepared = prepared.env(name, value);
    }
    prepared
}

fn unicode_value(field: &'static str, value: &OsStr) -> Result<String, RuntimeError> {
    value.to_str().map(str::to_owned).ok_or_else(|| {
        RuntimeError::new(
            RuntimeOperation::PrepareEnvironment,
            format!("{field} must be valid Unicode for SSH"),
        )
    })
}

fn ssh_destination(user: Option<&str>, host: &str) -> String {
    user.map_or_else(|| host.to_owned(), |user| format!("{user}@{host}"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn directory_health(path: &Path) -> EnvironmentHealth {
    match fs::read_dir(path) {
        Ok(_) => EnvironmentHealth::Healthy,
        Err(_) => EnvironmentHealth::InvalidWorkingDirectory,
    }
}

fn executable_exists(executable: &OsStr) -> bool {
    let path = Path::new(executable);
    if path.components().count() > 1 {
        return is_executable(path);
    }
    let Some(search_path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&search_path).any(|directory| executable_candidates(&directory, executable))
}

fn executable_candidates(directory: &Path, executable: &OsStr) -> bool {
    let candidate = directory.join(executable);
    if is_executable(&candidate) {
        return true;
    }
    #[cfg(windows)]
    {
        let extensions = env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
        extensions.to_string_lossy().split(';').any(|extension| {
            is_executable(&candidate.with_extension(extension.trim_start_matches('.')))
        })
    }
    #[cfg(not(windows))]
    false
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

enum ProbeError {
    Spawn(io::Error),
    TimedOut,
}

fn run_probe(mut command: Command) -> Result<Output, ProbeError> {
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(ProbeError::Spawn)?;
    let deadline = Instant::now() + HEALTH_TIMEOUT;
    loop {
        match child.try_wait().map_err(ProbeError::Spawn)? {
            Some(_) => return child.wait_with_output().map_err(ProbeError::Spawn),
            None if Instant::now() < deadline => thread::sleep(HEALTH_POLL_INTERVAL),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProbeError::TimedOut);
            }
        }
    }
}

fn authentication_failed(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    [
        "permission denied",
        "authentication failed",
        "no supported authentication methods",
        "publickey",
    ]
    .iter()
    .any(|marker| stderr.contains(marker))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use crate::domain::{
        ContainerEnvironment, CustomEnvironment, EnvironmentKind, EnvironmentProfile,
        EnvironmentProfileId, Name, SshEnvironment, WorkspaceDirectory,
    };

    use super::*;

    #[test]
    fn ssh_wraps_the_command_and_quotes_remote_values() {
        let profile = EnvironmentProfile::new(
            EnvironmentProfileId::new(1),
            Name::new("Remote").unwrap(),
            EnvironmentKind::Ssh(
                SshEnvironment::new(
                    "build.example.com",
                    Some("builder".to_owned()),
                    Some(2222),
                    "/work dir/it's",
                )
                .unwrap(),
            ),
        );
        let command = ProcessSpec::new("codex", "/local")
            .arg("$(touch /tmp/unsafe)")
            .env("TERM", "xterm-256color");

        let prepared = prepare_environment_process(Some(&profile), command).unwrap();

        assert_eq!(prepared.program(), OsStr::new("ssh"));
        assert_eq!(
            prepared.arguments(),
            [
                OsString::from("-tt"),
                OsString::from("-p"),
                OsString::from("2222"),
                OsString::from("builder@build.example.com"),
                OsString::from(
                    "'cd' '--' '/work dir/it'\"'\"'s' && 'exec' 'env' 'TERM=xterm-256color' 'codex' '$(touch /tmp/unsafe)'"
                ),
            ]
        );
    }

    #[test]
    fn container_preserves_the_command_as_arguments() {
        let profile = EnvironmentProfile::new(
            EnvironmentProfileId::new(1),
            Name::new("Container").unwrap(),
            EnvironmentKind::Container(
                ContainerEnvironment::new("podman", "dev", "/workspace").unwrap(),
            ),
        );
        let command = ProcessSpec::new("codex", "/local")
            .arg("--quiet")
            .env("TERM", "xterm-256color");

        let prepared = prepare_environment_process(Some(&profile), command).unwrap();

        assert_eq!(prepared.program(), OsStr::new("podman"));
        assert_eq!(
            prepared.arguments(),
            [
                "exec",
                "-it",
                "--workdir",
                "/workspace",
                "--env",
                "TERM=xterm-256color",
                "dev",
                "codex",
                "--quiet",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn custom_prefixes_the_command_without_a_shell() {
        let profile = EnvironmentProfile::new(
            EnvironmentProfileId::new(1),
            Name::new("Devbox").unwrap(),
            EnvironmentKind::Custom(
                CustomEnvironment::new(
                    "devbox",
                    vec!["run".to_owned(), "--".to_owned()],
                    WorkspaceDirectory::new("/custom").unwrap(),
                )
                .unwrap(),
            ),
        );

        let prepared = prepare_environment_process(
            Some(&profile),
            ProcessSpec::new("claude", "/local").arg("--debug"),
        )
        .unwrap();

        assert_eq!(prepared.program(), OsStr::new("devbox"));
        assert_eq!(prepared.working_directory(), Path::new("/custom"));
        assert_eq!(
            prepared.arguments(),
            ["run", "--", "claude", "--debug"].map(OsString::from)
        );
    }

    #[test]
    fn local_and_custom_health_validate_without_executing_the_command() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            check_environment(None, directory.path()),
            EnvironmentHealth::Healthy
        );
        let profile = EnvironmentProfile::new(
            EnvironmentProfileId::new(1),
            Name::new("Missing").unwrap(),
            EnvironmentKind::Custom(
                CustomEnvironment::new(
                    "openpodium-program-that-does-not-exist",
                    Vec::new(),
                    WorkspaceDirectory::new(directory.path().to_string_lossy()).unwrap(),
                )
                .unwrap(),
            ),
        );
        assert_eq!(
            check_environment(Some(&profile), directory.path()),
            EnvironmentHealth::MissingExecutable
        );
    }

    #[test]
    fn authentication_output_is_classified_without_becoming_an_error_message() {
        assert!(authentication_failed(
            b"Permission denied (publickey,password). secret prompt omitted"
        ));
        assert!(!authentication_failed(b"connection refused"));
        assert_eq!(
            EnvironmentHealth::AuthenticationFailed.label(),
            "authentication failed"
        );
    }
}
