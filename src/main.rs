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
        (Some(argument), None) if argument == "--native-tab-activation-smoke" => {
            finish(zorya::run_native_tab_activation_smoke())
        }
        (Some(argument), None) if argument == "--native-tab-close-smoke" => {
            finish(zorya::run_native_tab_close_smoke())
        }
        (Some(argument), None) if argument == "--native-tab-supersession-smoke" => {
            finish(zorya::run_native_tab_supersession_smoke())
        }
        (Some(argument), None) if argument == "--native-http-navigation-smoke" => {
            finish(zorya::run_native_http_navigation_smoke())
        }
        (Some(argument), None) if argument == "--native-profile-cycle-smoke" => {
            finish(zorya::run_native_profile_cycle_smoke())
        }
        (Some(argument), None) if argument == "--native-color-scheme-smoke" => {
            finish(zorya::run_native_color_scheme_smoke())
        }
        (Some(argument), None) if argument == "--native-bookmarks-persistence-smoke" => {
            finish(zorya::run_native_bookmarks_persistence_smoke())
        }
        (Some(argument), None) if argument == "--native-bookmark-toggle-add-smoke" => {
            finish(zorya::run_native_bookmark_toggle_add_smoke())
        }
        (Some(argument), None) if argument == "--native-bookmark-toggle-remove-smoke" => {
            finish(zorya::run_native_bookmark_toggle_remove_smoke())
        }
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
