//! Zippy — entry point.
//!
//! Binary yang sama melayani GUI dan verb command-line (dipakai context menu,
//! lihat Planning Doc §6.1). Sprint 0: hanya GUI (window kosong) untuk mengukur
//! baseline RSS/startup; dispatch verb CLI menyusul di v0.4 (Sprint 8-9).

mod cli;
mod file_list;
mod progress;
mod window;

use adw::prelude::*;
use libadwaita as adw;

const APP_ID: &str = "io.github.s4rt4.Zippy";

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Verb CLI (context menu): verb batch dijalankan tanpa GUI; path polos /
    // --open meneruskan ke GUI dengan archive yang diminta.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::dispatch(&args) {
        cli::Dispatch::Handled(code) => return code,
        cli::Dispatch::Gui(maybe_archive) => {
            if let Some(path) = maybe_archive {
                window::set_initial_archive(path);
            }
        }
        cli::Dispatch::GuiCompress(inputs) => {
            if !inputs.is_empty() {
                window::set_initial_compress(inputs);
            }
        }
    }

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(window::build_ui);

    // Jangan teruskan argv ke GTK (kita parse sendiri di cli.rs).
    let empty: [&str; 0] = [];
    let status = app.run_with_args(&empty);

    match status.value() {
        0 => std::process::ExitCode::SUCCESS,
        n => std::process::ExitCode::from(n as u8),
    }
}
