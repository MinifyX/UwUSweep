// Keep the console window away on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // The elevated helper is this same binary: one job, no window, no log.
    if args.get(1).map(String::as_str) == Some(uwusweep_core::helper::HELPER_FLAG) {
        std::process::exit(uwusweep_core::helper::helper_main(&args));
    }
    uwusweep_desktop_lib::run()
}
