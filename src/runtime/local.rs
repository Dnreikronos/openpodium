use std::io::{self, Read, Write};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc as std_mpsc};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use portable_pty::ChildKiller;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{mpsc, oneshot};

use super::{
    ProcessEvent, ProcessExit, ProcessRuntime, ProcessSpec, ProcessTermination, RuntimeError,
    RuntimeOperation, TerminalSize,
};

const OUTPUT_CHUNK_SIZE: usize = 8 * 1024;
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);
const RUNNING: u8 = 0;
const CANCELLATION_REQUESTED: u8 = 1;
const FINISHED: u8 = 2;

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalProcessRuntime;

impl ProcessRuntime for LocalProcessRuntime {
    fn spawn(&self, spec: ProcessSpec) -> Result<RunningProcess, RuntimeError> {
        spec.validate()?;

        let pair = native_pty_system()
            .openpty(spec.terminal_size().into())
            .map_err(|error| RuntimeError::new(RuntimeOperation::OpenPty, error.to_string()))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| RuntimeError::new(RuntimeOperation::CloneReader, error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| RuntimeError::new(RuntimeOperation::TakeWriter, error.to_string()))?;

        let mut command = CommandBuilder::new(spec.program());
        command.args(spec.arguments());
        command.cwd(spec.working_directory());
        for (name, value) in spec.environment() {
            command.env(name, value);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| RuntimeError::new(RuntimeOperation::Spawn, error.to_string()))?;
        drop(pair.slave);

        let process_id = child.process_id();
        #[cfg(unix)]
        let native_process_id = process_id.and_then(|id| i32::try_from(id).ok());
        #[cfg(windows)]
        let native_process_handle = match clone_process_handle(child.as_ref()) {
            Ok(handle) => handle,
            Err(error) => {
                let mut failed_child = child;
                let _ = failed_child.kill();
                let _ = failed_child.wait();
                return Err(RuntimeError::new(
                    RuntimeOperation::Spawn,
                    error.to_string(),
                ));
            }
        };
        let control = Arc::new(ProcessControl {
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(Some(writer)),
            #[cfg(unix)]
            killer: Mutex::new(child.clone_killer()),
            lifecycle: Mutex::new(()),
            state: AtomicU8::new(RUNNING),
            #[cfg(unix)]
            native_process_id,
            #[cfg(windows)]
            native_process_handle,
        });
        let (output_sender, mut output) = mpsc::channel(spec.output_capacity());
        let (termination_sender, termination) = oneshot::channel();
        let (reader_done_sender, reader_done) = std_mpsc::channel();
        let child_slot = Arc::new(Mutex::new(Some(child)));

        let waiter_control = Arc::clone(&control);
        let waiter_child = Arc::clone(&child_slot);
        let waiter = thread::Builder::new()
            .name("openpodium-pty-waiter".to_owned())
            .spawn(move || {
                let _worker = WorkerGuard::new();
                let child = lock(&waiter_child, RuntimeOperation::Wait).and_then(|mut slot| {
                    slot.take().ok_or_else(|| {
                        RuntimeError::new(
                            RuntimeOperation::Wait,
                            "the child process handle is unavailable",
                        )
                    })
                });
                let waited = match child {
                    Ok(child) => wait_for_child(child, &waiter_control),
                    Err(error) => {
                        let _ = waiter_control.finish_waiting();
                        Err(error)
                    }
                };
                let close_result = waiter_control.close_pty(RuntimeOperation::Wait);
                let reader_result = reader_done.recv().unwrap_or_else(|_| {
                    Err(RuntimeError::new(
                        RuntimeOperation::ReadOutput,
                        "the output reader stopped without reporting its result",
                    ))
                });
                let termination = match (waited, close_result, reader_result) {
                    (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                        ProcessTermination::Failed(error)
                    }
                    (Ok((exit, true)), Ok(()), Ok(())) => ProcessTermination::Cancelled(exit),
                    (Ok((exit, false)), Ok(()), Ok(())) => ProcessTermination::Exited(exit),
                };
                let _ = termination_sender.send(termination);
            })
            .map_err(|error| {
                let _ = control.cancel();
                cleanup_unstarted_child(&child_slot);
                RuntimeError::new(RuntimeOperation::SpawnWorker, error.to_string())
            })?;

        let reader_result = thread::Builder::new()
            .name("openpodium-pty-reader".to_owned())
            .spawn(move || {
                let _worker = WorkerGuard::new();
                let result = read_output(reader, output_sender);
                let _ = reader_done_sender.send(result);
            });
        if let Err(error) = reader_result {
            output.close();
            let _ = control.cancel();
            let _ = waiter.join();
            return Err(RuntimeError::new(
                RuntimeOperation::SpawnWorker,
                error.to_string(),
            ));
        }

        Ok(RunningProcess {
            process_id,
            control,
            output,
            termination: Some(termination),
        })
    }
}

pub struct RunningProcess {
    process_id: Option<u32>,
    control: Arc<ProcessControl>,
    output: mpsc::Receiver<Vec<u8>>,
    termination: Option<oneshot::Receiver<ProcessTermination>>,
}

impl RunningProcess {
    pub const fn process_id(&self) -> Option<u32> {
        self.process_id
    }

    pub fn write_input(&self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.control.ensure_running(RuntimeOperation::WriteInput)?;
        let mut writer_handle = lock(&self.control.writer, RuntimeOperation::WriteInput)?;
        let writer = writer_handle.as_mut().ok_or_else(|| {
            RuntimeError::new(RuntimeOperation::WriteInput, "the process is not running")
        })?;
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(|error| RuntimeError::new(RuntimeOperation::WriteInput, error.to_string()))
    }

    pub fn resize(&self, size: TerminalSize) -> Result<(), RuntimeError> {
        self.control.ensure_running(RuntimeOperation::Resize)?;
        let master_handle = lock(&self.control.master, RuntimeOperation::Resize)?;
        let master = master_handle.as_ref().ok_or_else(|| {
            RuntimeError::new(RuntimeOperation::Resize, "the process is not running")
        })?;
        master
            .resize(size.into())
            .map_err(|error| RuntimeError::new(RuntimeOperation::Resize, error.to_string()))
    }

    pub fn cancel(&mut self) -> Result<(), RuntimeError> {
        self.output.close();
        self.control.cancel()
    }

    pub async fn next_event(&mut self) -> Option<ProcessEvent> {
        if let Some(output) = self.output.recv().await {
            return Some(ProcessEvent::Output(output));
        }
        let termination = self.termination.take()?;
        Some(ProcessEvent::Terminated(termination.await.unwrap_or_else(
            |_| {
                ProcessTermination::Failed(RuntimeError::new(
                    RuntimeOperation::Wait,
                    "the process waiter stopped before reporting termination",
                ))
            },
        )))
    }
}

impl Drop for RunningProcess {
    fn drop(&mut self) {
        self.output.close();
        let _ = self.control.cancel();
    }
}

struct ProcessControl {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    #[cfg(unix)]
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    // A PID is safe to signal only until the waiter reaps it. This lock keeps
    // that transition atomic with cancellation on every platform.
    lifecycle: Mutex<()>,
    state: AtomicU8,
    #[cfg(unix)]
    native_process_id: Option<i32>,
    #[cfg(windows)]
    native_process_handle: OwnedHandle,
}

impl ProcessControl {
    fn ensure_running(&self, operation: RuntimeOperation) -> Result<(), RuntimeError> {
        if self.state.load(Ordering::Acquire) == RUNNING {
            Ok(())
        } else {
            Err(RuntimeError::new(operation, "the process is not running"))
        }
    }

    fn cancel(&self) -> Result<(), RuntimeError> {
        match self.state.compare_exchange(
            RUNNING,
            CANCELLATION_REQUESTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) | Err(CANCELLATION_REQUESTED) => {}
            Err(FINISHED) => return Ok(()),
            Err(_) => {
                return Err(RuntimeError::new(
                    RuntimeOperation::Cancel,
                    "the process has an unknown runtime state",
                ));
            }
        }

        {
            let lifecycle = lock(&self.lifecycle, RuntimeOperation::Cancel)?;
            if self.state.load(Ordering::Acquire) == FINISHED {
                return Ok(());
            }
            self.kill_process(&lifecycle)?;
        }
        self.close_pty(RuntimeOperation::Cancel)
    }

    fn close_pty(&self, operation: RuntimeOperation) -> Result<(), RuntimeError> {
        let writer = lock(&self.writer, operation)?.take();
        let master = lock(&self.master, operation)?.take();
        drop(writer);
        drop(master);
        Ok(())
    }

    fn kill_process(&self, _lifecycle: &MutexGuard<'_, ()>) -> Result<(), RuntimeError> {
        #[cfg(windows)]
        {
            let result = unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(
                    self.native_process_handle.as_raw_handle(),
                    1,
                )
            };
            if result != 0 || self.state.load(Ordering::Acquire) == FINISHED {
                return Ok(());
            }
            Err(RuntimeError::new(
                RuntimeOperation::Cancel,
                io::Error::last_os_error().to_string(),
            ))
        }

        #[cfg(unix)]
        if let Some(process_id) = self.native_process_id {
            // SAFETY: the child creates a new session before exec, making its
            // positive PID and negative process-group ID valid kill targets.
            unsafe {
                libc::kill(-process_id, libc::SIGKILL);
            }
            // SAFETY: the PID comes directly from the still-owned child handle.
            let result = unsafe { libc::kill(process_id, libc::SIGKILL) };
            if result == 0 || self.state.load(Ordering::Acquire) == FINISHED {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(RuntimeError::new(
                RuntimeOperation::Cancel,
                error.to_string(),
            ));
        }

        #[cfg(unix)]
        match lock(&self.killer, RuntimeOperation::Cancel)?.kill() {
            Ok(()) => Ok(()),
            Err(_) if self.state.load(Ordering::Acquire) == FINISHED => Ok(()),
            Err(error) => Err(RuntimeError::new(
                RuntimeOperation::Cancel,
                error.to_string(),
            )),
        }
    }

    fn finish_waiting(&self) -> Result<bool, RuntimeError> {
        let lifecycle = lock(&self.lifecycle, RuntimeOperation::Wait)?;
        Ok(self.finish_locked(&lifecycle))
    }

    fn finish_locked(&self, _lifecycle: &MutexGuard<'_, ()>) -> bool {
        self.state.swap(FINISHED, Ordering::AcqRel) == CANCELLATION_REQUESTED
    }

    fn try_wait_child(
        &self,
        child: &mut dyn Child,
    ) -> Result<Option<(ProcessExit, bool)>, RuntimeError> {
        let lifecycle = lock(&self.lifecycle, RuntimeOperation::Wait)?;
        match child.try_wait() {
            Ok(Some(status)) => Ok(Some((
                ProcessExit {
                    code: status.exit_code(),
                    signal: status.signal().map(ToOwned::to_owned),
                    success: status.success(),
                },
                self.finish_locked(&lifecycle),
            ))),
            Ok(None) => Ok(None),
            Err(error) => {
                self.finish_locked(&lifecycle);
                Err(RuntimeError::new(RuntimeOperation::Wait, error.to_string()))
            }
        }
    }
}

#[cfg(windows)]
fn clone_process_handle(child: &dyn Child) -> io::Result<OwnedHandle> {
    let raw_handle = child.as_raw_handle().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the child process handle is unavailable",
        )
    })?;
    let borrowed = unsafe { BorrowedHandle::borrow_raw(raw_handle) };
    borrowed.try_clone_to_owned()
}

fn wait_for_child(
    mut child: Box<dyn Child + Send + Sync>,
    control: &ProcessControl,
) -> Result<(ProcessExit, bool), RuntimeError> {
    loop {
        if let Some(termination) = control.try_wait_child(child.as_mut())? {
            return Ok(termination);
        }
        thread::sleep(CHILD_POLL_INTERVAL);
    }
}

fn read_output(
    mut reader: Box<dyn Read + Send>,
    output: mpsc::Sender<Vec<u8>>,
) -> Result<(), RuntimeError> {
    let mut buffer = vec![0; OUTPUT_CHUNK_SIZE];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if output.blocking_send(buffer[..count].to_vec()).is_err() {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if is_pty_end(&error) => return Ok(()),
            Err(error) => {
                return Err(RuntimeError::new(
                    RuntimeOperation::ReadOutput,
                    error.to_string(),
                ));
            }
        }
    }
}

fn is_pty_end(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof
    ) || cfg!(unix) && error.raw_os_error() == Some(5)
}

fn cleanup_unstarted_child(child_slot: &Mutex<Option<Box<dyn Child + Send + Sync>>>) {
    let Ok(mut slot) = child_slot.lock() else {
        return;
    };
    let Some(mut child) = slot.take() else {
        return;
    };
    let _ = child.kill();
    let _ = child.wait();
}

fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    operation: RuntimeOperation,
) -> Result<MutexGuard<'a, T>, RuntimeError> {
    mutex
        .lock()
        .map_err(|_| RuntimeError::new(operation, "a runtime lock was poisoned"))
}

impl From<TerminalSize> for PtySize {
    fn from(size: TerminalSize) -> Self {
        Self {
            rows: size.rows(),
            cols: size.columns(),
            pixel_width: size.pixel_width(),
            pixel_height: size.pixel_height(),
        }
    }
}

struct WorkerGuard;

impl WorkerGuard {
    fn new() -> Self {
        #[cfg(test)]
        ACTIVE_WORKERS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        #[cfg(test)]
        ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
static ACTIVE_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub(super) fn active_workers() -> usize {
    ACTIVE_WORKERS.load(Ordering::Acquire)
}
