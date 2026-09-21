use std::path::Path;
use std::sync::Arc;

use openpodium::domain::{AgentProgram, CommandPreset, Role};
use openpodium::runtime::{
    AgentAdapterError, ProcessController, ProcessEvent, ProcessSpec, ProcessTermination,
    RunningProcess, TerminalSize, prepare_agent_process,
};
use tokio::sync::Mutex;

use super::{GridSize, InputMode, Model, Status, Update, View, encode_paste};

pub(crate) type ProcessStream = Arc<Mutex<RunningProcess>>;

pub(crate) enum Action {
    ClipboardStore(String),
    Bell,
}

/// How much recent output a node keeps so it can reopen showing its screen.
/// Bounded because a transcript is a display cache, not a log: the tail is the
/// only part that still tells you where you were.
const TRANSCRIPT_LIMIT: usize = 128 * 1024;

pub(crate) struct Session {
    model: Model,
    status: Status,
    controller: Option<ProcessController>,
    stream: Option<ProcessStream>,
    generation: u64,
    transcript: Vec<u8>,
    transcript_dirty: bool,
}

impl Session {
    pub(crate) fn starting(size: GridSize, generation: u64) -> Self {
        Self {
            model: Model::new(size),
            status: Status::Starting,
            controller: None,
            stream: None,
            generation,
            transcript: Vec::new(),
            transcript_dirty: false,
        }
    }

    /// Rebuilds a node's last screen from a stored transcript. The process is
    /// gone, so the session stays offline and only replays what it showed.
    pub(crate) fn restored(size: GridSize, generation: u64, transcript: Vec<u8>) -> Self {
        let mut model = Model::new(size);
        let _ = model.feed(&transcript);
        Self {
            model,
            status: Status::Offline,
            controller: None,
            stream: None,
            generation,
            transcript,
            transcript_dirty: false,
        }
    }

    /// The bytes worth storing, or `None` when nothing has changed since the
    /// last capture.
    pub(crate) fn take_transcript(&mut self) -> Option<&[u8]> {
        self.transcript_dirty.then(|| {
            self.transcript_dirty = false;
            self.transcript.as_slice()
        })
    }

    fn record(&mut self, bytes: &[u8]) {
        self.transcript.extend_from_slice(bytes);
        if self.transcript.len() > TRANSCRIPT_LIMIT {
            // Keep the tail, and start it at a line boundary so the replay does
            // not begin partway through an escape sequence.
            let excess = self.transcript.len() - TRANSCRIPT_LIMIT;
            let start = self.transcript[excess..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(excess, |offset| excess + offset + 1);
            self.transcript.drain(..start);
        }
        self.transcript_dirty = true;
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn attach(&mut self, stream: ProcessStream) -> Result<(), &'static str> {
        let controller = stream
            .try_lock()
            .map_err(|_| "the process stream was busy before event polling started")?
            .controller();
        self.controller = Some(controller);
        self.stream = Some(stream);
        self.status = Status::Running;
        Ok(())
    }

    pub(crate) fn fail(&mut self, error: impl ToString) {
        self.controller = None;
        self.stream = None;
        self.status = Status::Failed(error.to_string());
    }

    pub(crate) fn stream(&self) -> Option<ProcessStream> {
        self.stream.clone()
    }

    pub(crate) fn is_active(&self) -> bool {
        matches!(self.status, Status::Starting | Status::Running)
    }

    pub(crate) fn view(&self) -> View {
        self.model.view(self.status.clone())
    }

    pub(crate) fn input_mode(&self) -> InputMode {
        self.model.input_mode()
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), String> {
        self.controller
            .as_ref()
            .ok_or_else(|| "the terminal process is not running".to_owned())?
            .write_input(bytes)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn paste(&self, text: &str) -> Result<(), String> {
        self.write(&encode_paste(text, self.input_mode()))
    }

    pub(crate) fn resize(&mut self, size: GridSize) -> Result<(), String> {
        self.model.resize(size);
        let Some(controller) = &self.controller else {
            return Ok(());
        };
        controller
            .resize(
                TerminalSize::new(size.rows, size.columns)
                    .expect("terminal grid dimensions are always non-zero"),
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn scroll(&mut self, lines: i32) {
        self.model.scroll(lines);
    }

    pub(crate) fn begin_selection(&mut self, row: usize, column: usize, right_side: bool) {
        self.model.begin_selection(row, column, right_side);
    }

    pub(crate) fn update_selection(&mut self, row: usize, column: usize, right_side: bool) {
        self.model.update_selection(row, column, right_side);
    }

    pub(crate) fn clear_selection(&mut self) {
        self.model.clear_selection();
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        self.model.selected_text()
    }

    pub(crate) fn handle_event(&mut self, event: ProcessEvent) -> Vec<Action> {
        match event {
            ProcessEvent::Output(bytes) => {
                self.record(&bytes);
                self.model
                    .feed(&bytes)
                    .into_iter()
                    .filter_map(|update| match update {
                        Update::PtyWrite(bytes) => {
                            if let Err(error) = self.write(&bytes) {
                                self.status = Status::Failed(error.to_string());
                            }
                            None
                        }
                        Update::ClipboardStore(text) => Some(Action::ClipboardStore(text)),
                        Update::Bell => Some(Action::Bell),
                        Update::TitleChanged(title) => {
                            let _ = title;
                            None
                        }
                    })
                    .collect()
            }
            ProcessEvent::Terminated(termination) => {
                self.controller = None;
                self.stream = None;
                self.status = match termination {
                    ProcessTermination::Exited(exit) => {
                        let detail = exit
                            .signal()
                            .map_or_else(|| exit.code().to_string(), |signal| signal.to_owned());
                        Status::Exited(detail)
                    }
                    ProcessTermination::Cancelled(_) => Status::Stopped,
                    ProcessTermination::Failed(error) => Status::Failed(error.to_string()),
                };
                Vec::new()
            }
        }
    }

    pub(crate) fn stop(&mut self) -> Result<(), String> {
        if let Some(controller) = self.controller.take() {
            controller.cancel().map_err(|error| error.to_string())?;
        }
        self.stream = None;
        if self.is_active() {
            self.status = Status::Stopped;
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub(crate) fn process_spec(
    program: AgentProgram,
    preset: Option<&CommandPreset>,
    role: Option<&Role>,
    working_directory: &Path,
    size: GridSize,
) -> Result<ProcessSpec, AgentAdapterError> {
    prepare_agent_process(
        program,
        preset,
        role,
        working_directory,
        TerminalSize::new(size.rows, size.columns)
            .expect("terminal grid dimensions are always non-zero"),
    )
}

#[cfg(test)]
mod tests {
    use super::{Session, TRANSCRIPT_LIMIT};
    use crate::terminal::{GridSize, Status};
    use openpodium::runtime::ProcessEvent;

    fn size() -> GridSize {
        GridSize::for_node(400.0, 300.0)
    }

    #[test]
    fn a_restored_session_replays_its_transcript_without_running_anything() {
        let session = Session::restored(size(), 1, b"hello".to_vec());
        let view = session.view();

        assert!(!session.is_active());
        assert!(matches!(view.status, Status::Offline));
        assert!(
            view.cells.iter().any(|cell| cell.text == "h"),
            "the stored output should be back on screen"
        );
    }

    #[test]
    fn a_transcript_is_captured_once_per_change() {
        let mut session = Session::starting(size(), 1);
        assert!(session.take_transcript().is_none());

        session.handle_event(ProcessEvent::Output(b"first".to_vec()));
        assert_eq!(session.take_transcript(), Some(b"first".as_slice()));
        assert!(session.take_transcript().is_none());

        session.handle_event(ProcessEvent::Output(b" second".to_vec()));
        assert_eq!(session.take_transcript(), Some(b"first second".as_slice()));
    }

    /// The buffer is bounded, and trimming starts at a line boundary so the
    /// replay never begins partway through an escape sequence.
    #[test]
    fn an_overlong_transcript_keeps_whole_lines_from_the_tail() {
        let mut session = Session::starting(size(), 1);
        let line = b"0123456789abcdef\n";
        let repeats = TRANSCRIPT_LIMIT / line.len() + 8;
        for _ in 0..repeats {
            session.handle_event(ProcessEvent::Output(line.to_vec()));
        }

        let transcript = session.take_transcript().unwrap();
        assert!(transcript.len() <= TRANSCRIPT_LIMIT);
        assert!(
            transcript.starts_with(b"0123456789abcdef"),
            "a trimmed transcript should resume at the start of a line"
        );
        assert!(transcript.ends_with(b"\n"));
    }
}
