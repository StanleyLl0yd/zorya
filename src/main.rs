use std::process::ExitCode;

fn main() -> ExitCode {
    match zorya::ZoryaApp::bootstrap() {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zorya: {error}");
            ExitCode::FAILURE
        }
    }
}
