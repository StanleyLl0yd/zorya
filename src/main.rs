use std::process::ExitCode;

fn main() -> ExitCode {
    match zorya::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zorya: {error}");
            ExitCode::FAILURE
        }
    }
}
