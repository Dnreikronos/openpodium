use std::path::Path;
use std::time::Duration;

use tempfile::tempdir;
use tokio::time::{Instant, sleep, timeout, timeout_at};

use super::local::active_workers;
use super::{
    LocalProcessRuntime, ProcessEvent, ProcessRuntime, ProcessSpec, ProcessTermination,
    RuntimeOperation, TerminalSize,
};

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test(flavor = "current_thread")]
async fn interactive_shell_uses_working_directory_and_accepts_input() {
    let _test = TEST_LOCK.lock().await;
    let directory = tempdir().expect("temporary working directory");
    let expected_directory = directory.path().canonicalize().expect("canonical path");
    let mut process = LocalProcessRuntime
        .spawn(interactive_shell(directory.path()))
        .expect("interactive shell starts");

    let mut output = initialize_terminal_host(&mut process).await;
    process
        .write_input(shell_input_for_working_directory())
        .expect("input reaches shell");
    output.extend(collect_until_output(&mut process, b"__OPENPODIUM_READY__").await);
    process
        .write_input(shell_exit_input())
        .expect("exit reaches shell");
    let (remaining_output, termination) = collect_process(&mut process).await;
    output.extend(remaining_output);

    let output = String::from_utf8_lossy(&output);
    let expected_directory = expected_directory.to_string_lossy();
    let expected_directory = expected_directory
        .strip_prefix(r"\\?\")
        .unwrap_or(&expected_directory);
    assert!(output.contains("__OPENPODIUM_READY__"), "output: {output}");
    assert!(
        output.contains(&expected_directory.replace('\\', "/"))
            || output.contains(expected_directory),
        "output: {output}"
    );
    assert!(matches!(
        termination,
        ProcessTermination::Exited(exit) if exit.success()
    ));
    assert_eq!(process.next_event().await, None);
    drop(process);
    wait_for_workers_to_stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn resize_is_visible_inside_the_child_terminal() {
    let _test = TEST_LOCK.lock().await;
    let directory = tempdir().expect("temporary working directory");
    let size = TerminalSize::new(41, 111).expect("valid terminal size");
    let mut process = LocalProcessRuntime
        .spawn(interactive_shell(directory.path()))
        .expect("interactive shell starts");

    let _ = initialize_terminal_host(&mut process).await;
    process.resize(size).expect("PTY resizes");
    process
        .write_input(shell_input_for_terminal_size())
        .expect("terminal query reaches shell");
    let mut output = collect_until_output(&mut process, b"41 111").await;
    process
        .write_input(shell_exit_input())
        .expect("exit reaches shell");
    let (remaining_output, termination) = collect_process(&mut process).await;
    output.extend(remaining_output);

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("41 111"), "output: {output}");
    assert!(matches!(
        termination,
        ProcessTermination::Exited(exit) if exit.success()
    ));
    drop(process);
    wait_for_workers_to_stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn exit_and_startup_failure_are_each_observed_once() {
    let _test = TEST_LOCK.lock().await;
    let directory = tempdir().expect("temporary working directory");
    let error = match LocalProcessRuntime.spawn(ProcessSpec::new(
        "openpodium-program-that-does-not-exist",
        directory.path(),
    )) {
        Ok(_) => panic!("invalid executable unexpectedly started"),
        Err(error) => error,
    };
    assert_eq!(error.operation(), RuntimeOperation::Spawn);

    let mut process = LocalProcessRuntime
        .spawn(exiting_process(directory.path(), 7))
        .expect("child starts");
    let _ = initialize_terminal_host(&mut process).await;
    let (_, termination) = collect_process(&mut process).await;
    assert!(
        matches!(
            termination,
            ProcessTermination::Exited(ref exit) if exit.code() == 7 && !exit.success()
        ),
        "termination: {termination:?}"
    );
    assert_eq!(process.next_event().await, None);
    drop(process);
    wait_for_workers_to_stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_releases_a_reader_blocked_by_backpressure() {
    let _test = TEST_LOCK.lock().await;
    let directory = tempdir().expect("temporary working directory");
    let mut process = LocalProcessRuntime
        .spawn(noisy_process(directory.path()).with_output_capacity(1))
        .expect("noisy child starts");

    sleep(Duration::from_millis(100)).await;
    process.cancel().expect("child is cancelled");
    let (_, termination) = collect_process(&mut process).await;
    assert!(matches!(termination, ProcessTermination::Cancelled(_)));
    drop(process);
    wait_for_workers_to_stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_a_handle_cancels_its_child_and_workers() {
    let _test = TEST_LOCK.lock().await;
    let directory = tempdir().expect("temporary working directory");
    let process = LocalProcessRuntime
        .spawn(noisy_process(directory.path()).with_output_capacity(1))
        .expect("noisy child starts");

    sleep(Duration::from_millis(100)).await;
    assert_eq!(active_workers(), 2);
    drop(process);
    wait_for_workers_to_stop().await;
}

async fn collect_process(process: &mut super::RunningProcess) -> (Vec<u8>, ProcessTermination) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    let mut output = Vec::new();
    loop {
        match timeout_at(deadline, process.next_event()).await {
            Ok(Some(ProcessEvent::Output(chunk))) => output.extend(chunk),
            Ok(Some(ProcessEvent::Terminated(termination))) => return (output, termination),
            Ok(None) => panic!("process event stream ended without termination"),
            Err(_) => panic!(
                "process did not terminate; lifecycle: {}; output: {}",
                process.lifecycle_state(),
                String::from_utf8_lossy(&output)
            ),
        }
    }
}

async fn collect_until_output(process: &mut super::RunningProcess, expected: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + TEST_TIMEOUT;
    let mut output = Vec::new();
    loop {
        match timeout_at(deadline, process.next_event()).await {
            Ok(Some(ProcessEvent::Output(chunk))) => {
                output.extend(chunk);
                if output
                    .windows(expected.len())
                    .any(|window| window == expected)
                {
                    return output;
                }
            }
            Ok(Some(ProcessEvent::Terminated(termination))) => {
                panic!(
                    "process terminated before expected output: {termination:?}; output: {}",
                    String::from_utf8_lossy(&output)
                );
            }
            Ok(None) => panic!("process event stream ended before expected output"),
            Err(_) => panic!(
                "process did not produce expected output; lifecycle: {}; output: {}",
                process.lifecycle_state(),
                String::from_utf8_lossy(&output)
            ),
        }
    }
}

async fn wait_for_workers_to_stop() {
    timeout(TEST_TIMEOUT, async {
        while active_workers() != 0 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("runtime workers stop before timeout");
}

#[cfg(unix)]
async fn initialize_terminal_host(_process: &mut super::RunningProcess) -> Vec<u8> {
    Vec::new()
}

#[cfg(windows)]
async fn initialize_terminal_host(process: &mut super::RunningProcess) -> Vec<u8> {
    // portable-pty requests cursor inheritance when it creates a ConPTY.
    // A real terminal emulator answers this query before the child can run.
    let output = collect_until_output(process, b"\x1b[6n").await;
    process
        .write_input(b"\x1b[1;1R")
        .expect("cursor position response reaches ConPTY");
    output
}

#[cfg(unix)]
fn interactive_shell(directory: &Path) -> ProcessSpec {
    ProcessSpec::new("/bin/sh", directory).env("PS1", "")
}

#[cfg(windows)]
fn interactive_shell(directory: &Path) -> ProcessSpec {
    ProcessSpec::new("cmd.exe", directory).arg("/Q")
}

#[cfg(unix)]
fn shell_input_for_working_directory() -> &'static [u8] {
    b"printf '__OPENPODIUM_READY__\\n'; pwd\n"
}

#[cfg(windows)]
fn shell_input_for_working_directory() -> &'static [u8] {
    b"echo __OPENPODIUM_READY__& cd\r"
}

#[cfg(unix)]
fn shell_input_for_terminal_size() -> &'static [u8] {
    b"stty size\n"
}

#[cfg(windows)]
fn shell_input_for_terminal_size() -> &'static [u8] {
    b"powershell.exe -NoLogo -NoProfile -Command \"$s=$Host.UI.RawUI.WindowSize; Write-Output ('{0} {1}' -f $s.Height,$s.Width)\"\r"
}

fn exiting_process(directory: &Path, code: u8) -> ProcessSpec {
    ProcessSpec::new(
        std::env::current_exe().expect("current test executable"),
        directory,
    )
    .args([
        "--exact",
        "runtime::tests::pty_child_exits_with_requested_code",
        "--ignored",
        "--nocapture",
    ])
    .env("OPENPODIUM_TEST_EXIT_CODE", code.to_string())
}

#[test]
#[ignore = "spawned as a PTY subprocess by the runtime tests"]
fn pty_child_exits_with_requested_code() {
    let code = std::env::var("OPENPODIUM_TEST_EXIT_CODE")
        .expect("requested exit code")
        .parse()
        .expect("numeric exit code");
    std::process::exit(code);
}

#[cfg(unix)]
fn shell_exit_input() -> &'static [u8] {
    b"exit 0\n"
}

#[cfg(windows)]
fn shell_exit_input() -> &'static [u8] {
    b"exit /b 0\r"
}

#[cfg(unix)]
fn noisy_process(directory: &Path) -> ProcessSpec {
    ProcessSpec::new("/bin/sh", directory).args([
        "-c",
        "while :; do printf '0123456789abcdef0123456789abcdef'; done",
    ])
}

#[cfg(windows)]
fn noisy_process(directory: &Path) -> ProcessSpec {
    ProcessSpec::new("powershell.exe", directory).args([
        "-NoLogo",
        "-NoProfile",
        "-Command",
        "while ($true) { [Console]::Write('0123456789abcdef0123456789abcdef') }",
    ])
}
