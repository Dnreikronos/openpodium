use std::path::Path;
use std::sync::{Arc, OnceLock};

use openpodium::domain::{AgentProgram, CommandPreset, Role};
use openpodium::runtime::{
    AgentAdapterError, ProcessController, ProcessEvent, ProcessSpec, ProcessTermination,
    RunningProcess, TerminalSize, prepare_agent_process,
};
use tokio::sync::Mutex;

use super::{DefaultColors, GridSize, InputMode, Model, Status, Update, View, encode_paste};

pub(crate) type ProcessStream = Arc<Mutex<RunningProcess>>;

pub(crate) enum Action {
    ClipboardStore(String),
    Bell,
}

pub(crate) struct Session {
    model: Model,
    status: Status,
    controller: Option<ProcessController>,
    stream: Option<ProcessStream>,
    generation: u64,
    transcript_dirty: bool,
    view_cache: OnceLock<View>,
}

impl Session {
    pub(crate) fn starting(size: GridSize, generation: u64) -> Self {
        Self {
            model: Model::new(size),
            status: Status::Starting,
            controller: None,
            stream: None,
            generation,
            transcript_dirty: false,
            view_cache: OnceLock::new(),
        }
    }

    /// Rebuilds a node's last screen from a stored transcript. The process is
    /// gone, so the session stays offline and only replays what it showed.
    pub(crate) fn restored(size: GridSize, generation: u64, transcript: Vec<u8>) -> Self {
        let model = super::snapshot::restore(size, &transcript);
        Self {
            model,
            status: Status::Offline,
            controller: None,
            stream: None,
            generation,
            transcript_dirty: false,
            view_cache: OnceLock::new(),
        }
    }

    /// The bytes worth storing, or `None` when nothing has changed since the
    /// successful store. Capturing does not acknowledge persistence.
    pub(crate) fn take_transcript(&mut self) -> Option<Vec<u8>> {
        self.transcript_dirty
            .then(|| super::snapshot::capture(&mut self.model))
    }

    pub(crate) fn mark_transcript_persisted(&mut self) {
        self.transcript_dirty = false;
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn prepare_restart(&mut self, size: GridSize, generation: u64) {
        self.view_cache.take();
        self.model.resize(size);
        self.generation = generation;
        self.status = Status::Starting;
    }

    pub(crate) fn attach(&mut self, stream: ProcessStream) -> Result<(), &'static str> {
        let controller = stream
            .try_lock()
            .map_err(|_| "the process stream was busy before event polling started")?
            .controller();
        self.model = Model::new(self.model.size);
        self.view_cache.take();
        self.controller = Some(controller);
        self.stream = Some(stream);
        self.status = Status::Running;
        Ok(())
    }

    pub(crate) fn fail(&mut self, error: impl ToString) {
        self.view_cache.take();
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
        self.view_cache
            .get_or_init(|| {
                let mut view = self.model.view(self.status.clone());
                view.mode.mouse_reporting &= matches!(self.status, Status::Running);
                view
            })
            .clone()
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
        if self.model.size != size {
            self.view_cache.take();
        }
        self.transcript_dirty |= self.model.size != size;
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
        self.view_cache.take();
        self.model.scroll(lines);
    }

    pub(crate) fn begin_selection(&mut self, row: usize, column: usize, right_side: bool) {
        self.view_cache.take();
        self.model.begin_selection(row, column, right_side);
    }

    pub(crate) fn update_selection(&mut self, row: usize, column: usize, right_side: bool) {
        self.view_cache.take();
        self.model.update_selection(row, column, right_side);
    }

    pub(crate) fn clear_selection(&mut self) {
        self.view_cache.take();
        self.model.clear_selection();
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        self.model.selected_text()
    }

    pub(crate) fn handle_event(
        &mut self,
        event: ProcessEvent,
        colors: DefaultColors,
    ) -> Vec<Action> {
        self.view_cache.take();
        match event {
            ProcessEvent::Output(bytes) => {
                self.transcript_dirty = true;
                self.model
                    .feed_with_colors(&bytes, colors)
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
        self.view_cache.take();
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
    use super::Session;
    use crate::terminal::{GridSize, Status};
    use openpodium::runtime::ProcessEvent;

    fn size() -> GridSize {
        GridSize::for_node(400.0, 300.0)
    }

    #[test]
    fn unchanged_views_share_cells_without_rebuilding_the_grid() {
        let mut session = Session::starting(size(), 1);
        session.handle_event(ProcessEvent::Output(b"hello".to_vec()), Default::default());
        let before = session.view();
        for _ in 0..100 {
            assert!(std::sync::Arc::ptr_eq(&before.cells, &session.view().cells));
        }
        session.resize(size()).unwrap();
        session.take_transcript();
        assert!(std::sync::Arc::ptr_eq(&before.cells, &session.view().cells));
    }

    #[test]
    fn cached_views_follow_every_screen_and_status_change() {
        fn verify(session: &Session) {
            let mut expected = session.model.view(session.status.clone());
            expected.mode.mouse_reporting &= matches!(session.status, Status::Running);
            assert_eq!(session.view(), expected);
        }

        let mut session = Session::starting(size(), 1);
        verify(&session);
        session.handle_event(
            ProcessEvent::Output(b"first\r\nsecond\r\n".repeat(40)),
            Default::default(),
        );
        verify(&session);
        session.scroll(5);
        verify(&session);
        session.begin_selection(0, 0, false);
        verify(&session);
        session.update_selection(1, 3, true);
        verify(&session);
        session.clear_selection();
        verify(&session);
        session
            .resize(GridSize {
                columns: 30,
                rows: 8,
            })
            .unwrap();
        verify(&session);
        session.stop().unwrap();
        verify(&session);
        session.prepare_restart(size(), 2);
        verify(&session);
        session.fail("test failure");
        verify(&session);
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
    fn failed_restart_preserves_the_saved_screen() {
        let mut session = Session::restored(size(), 1, b"saved conversation".to_vec());
        let before = session.view().cells;
        session.prepare_restart(size(), 2);
        assert!(session.is_active());
        assert_eq!(session.generation(), 2);
        session.fail("program could not be started");
        assert!(!session.is_active());
        assert_eq!(session.view().cells, before);
    }

    #[test]
    fn offline_mouse_reporting_uses_local_scrollback() {
        let mut session = Session::starting(size(), 1);
        session.handle_event(
            ProcessEvent::Output(b"\x1b[?1000hhello".to_vec()),
            Default::default(),
        );
        let payload = session.take_transcript().unwrap();
        let restored = Session::restored(size(), 2, payload);
        assert!(!restored.view().mode.mouse_reporting);
    }

    #[test]
    fn a_transcript_is_captured_once_per_change() {
        let mut session = Session::starting(size(), 1);
        assert!(session.take_transcript().is_none());

        session.handle_event(ProcessEvent::Output(b"first".to_vec()), Default::default());
        let first = session.take_transcript().unwrap();
        assert_eq!(session.take_transcript(), Some(first.clone()));
        session.mark_transcript_persisted();
        assert!(session.take_transcript().is_none());

        session.handle_event(
            ProcessEvent::Output(b" second".to_vec()),
            Default::default(),
        );
        assert_ne!(session.take_transcript(), Some(first));
    }

    #[test]
    fn large_output_preserves_early_styles_in_a_bounded_screen_snapshot() {
        let mut session = Session::starting(size(), 1);
        session.handle_event(
            ProcessEvent::Output(b"\x1b[31m".to_vec()),
            Default::default(),
        );
        let line = b"0123456789abcdef\r\n";
        for _ in 0..10_000 {
            session.handle_event(ProcessEvent::Output(line.to_vec()), Default::default());
        }
        let payload = session.take_transcript().unwrap();
        assert!(payload.len() < 1024 * 1024);
        let mut expected = session.view();
        expected.status = Status::Offline;
        assert_eq!(Session::restored(size(), 2, payload).view(), expected);
    }
}
