use std::path::Path;
use std::time::Duration;

use tempfile::tempdir;
use tokio::time::{sleep, timeout};

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

    process
        .write_input(shell_input_for_working_directory())
        .expect("input reaches shell");
    let (output, termination) = collect_process(&mut process).await;

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("__OPENPODIUM_READY__"), "output: {output}");
    assert!(
        output.contains(&expected_directory.to_string_lossy().replace('\\', "/"))
            || output.contains(&expected_directory.to_string_lossy().to_string()),
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

    process.resize(size).expect("PTY resizes");
    process
        .write_input(shell_input_for_terminal_size())
        .expect("terminal query reaches shell");
    let (output, termination) = collect_process(&mut process).await;

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
    let (_, termination) = collect_process(&mut process).await;
    assert!(matches!(
        termination,
        ProcessTermination::Exited(exit) if exit.code() == 7 && !exit.success()
    ));
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
    timeout(TEST_TIMEOUT, async {
        let mut output = Vec::new();
        loop {
            match process.next_event().await {
                Some(ProcessEvent::Output(chunk)) => output.extend(chunk),
                Some(ProcessEvent::Terminated(termination)) => return (output, termination),
                None => panic!("process event stream ended without termination"),
            }
        }
    })
    .await
    .expect("process terminates before timeout")
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
fn interactive_shell(directory: &Path) -> ProcessSpec {
    ProcessSpec::new("/bin/sh", directory).env("PS1", "")
}

#[cfg(windows)]
fn interactive_shell(directory: &Path) -> ProcessSpec {
    ProcessSpec::new("cmd.exe", directory).arg("/Q")
}

#[cfg(unix)]
fn shell_input_for_working_directory() -> &'static [u8] {
    b"printf '__OPENPODIUM_READY__\\n'; pwd; exit 0\n"
}

#[cfg(windows)]
fn shell_input_for_working_directory() -> &'static [u8] {
    b"echo __OPENPODIUM_READY__& cd & exit /b 0\r\n"
}

#[cfg(unix)]
fn shell_input_for_terminal_size() -> &'static [u8] {
    b"stty size; exit 0\n"
}

#[cfg(windows)]
fn shell_input_for_terminal_size() -> &'static [u8] {
    b"powershell.exe -NoLogo -NoProfile -Command \"$s=$Host.UI.RawUI.WindowSize; Write-Output ('{0} {1}' -f $s.Height,$s.Width)\" & exit /b 0\r\n"
}

#[cfg(unix)]
fn exiting_process(directory: &Path, code: u8) -> ProcessSpec {
    ProcessSpec::new("/bin/sh", directory)
        .arg("-c")
        .arg(format!("exit {code}"))
}

#[cfg(windows)]
fn exiting_process(directory: &Path, code: u8) -> ProcessSpec {
    ProcessSpec::new("cmd.exe", directory)
        .args(["/Q", "/C"])
        .arg(format!("exit /b {code}"))
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
