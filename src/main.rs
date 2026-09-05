use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();

    match (arguments.next(), arguments.next()) {
        (Some(argument), None) if argument == "--version" => {
            println!("Zorya {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        (None, None) => run_browser(),
        _ => {
            eprintln!("zorya: unsupported command-line arguments");
            ExitCode::FAILURE
        }
    }
}

fn run_browser() -> ExitCode {
    match zorya::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zorya: {error}");
            ExitCode::FAILURE
        }
    }
}
