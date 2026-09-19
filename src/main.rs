mod app;
mod canvas;
mod chat;
mod terminal;

fn main() -> iced::Result {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|argument| argument == "ipc") {
        std::process::exit(i32::from(openpodium::ipc::run_cli(arguments)));
    }
    app::run()
}
