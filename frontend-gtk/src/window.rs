//! Main window.
//!
//! Sprint 0: window kosong (header bar + placeholder) sebagai baseline untuk
//! mengukur RSS idle & cold-start GTK4+libadwaita (Planning Doc §9.1).
//! Layout nyata (toolbar + GtkColumnView + progress + status bar, §5.1) menyusul
//! di v0.2 (Sprint 4-5).

use std::time::Duration;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use libadwaita as adw;

pub fn build_ui(app: &adw::Application) {
    let header = adw::HeaderBar::new();

    let placeholder = adw::StatusPage::builder()
        .icon_name("package-x-generic-symbolic")
        .title("Zippy")
        .description("Archive manager — Sprint 0 baseline (window kosong)")
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&placeholder);
    placeholder.set_vexpand(true);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Zippy")
        .default_width(900)
        .default_height(600)
        .content(&content)
        .build();

    window.present();

    // Mode benchmark Sprint 0: biarkan window settle, laporkan RSS, lalu quit.
    // Dipakai oleh scripts/measure.sh. Aktif hanya bila ZIPPY_BENCH diset.
    if std::env::var_os("ZIPPY_BENCH").is_some() {
        let app = app.clone();
        glib::timeout_add_local_once(Duration::from_millis(800), move || {
            if let Some(kb) = read_vmrss_kb() {
                println!("ZIPPY_BENCH rss_kb={kb} rss_mb={:.1}", kb as f64 / 1024.0);
            } else {
                println!("ZIPPY_BENCH rss_kb=unknown");
            }
            app.quit();
        });
    }
}

/// Baca VmRSS proses ini (KB) dari /proc/self/status.
fn read_vmrss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}
