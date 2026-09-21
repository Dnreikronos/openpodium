mod app;
mod canvas;
mod chat;
mod navigation_panel;
mod notifications;
mod routines_panel;
mod supervisor_panel;
mod terminal;
mod timeline_panel;

fn main() -> iced::Result {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|argument| argument == "ipc") {
        std::process::exit(i32::from(openpodium::ipc::run_cli(arguments)));
    }
    app::run()
}
