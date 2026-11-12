// Qust (see lib.rs for how it is put together).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = qust_app::run(qust_app::Config::from_args(&args));
    std::process::exit(code);
}
