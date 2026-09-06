use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();

    match (arguments.next(), arguments.next()) {
        (Some(argument), None) if argument == "--version" => {
            println!("Zorya {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        (Some(argument), None) if argument == "--native-smoke" => finish(zorya::run_native_smoke()),
        (None, None) => finish(zorya::run()),
        _ => {
            eprintln!("zorya: unsupported command-line arguments");
            ExitCode::FAILURE
        }
    }
}

fn finish(result: Result<(), Box<dyn std::error::Error>>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zorya: {error}");
            ExitCode::FAILURE
        }
    }
}
