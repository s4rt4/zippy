//! Main window — v0.2 GTK4 Basic.
//!
//! Layout nyata (Planning Doc §5.1): AdwToolbarView dengan HeaderBar (Buka /
//! Extract / Tambah), GtkColumnView berisi daftar entry, dan progress bar dalam
//! revealer di bawah. Operasi berat (list/extract/compress) dijalankan di
//! `std::thread`; progress di-marshal balik ke UI lewat `async-channel` +
//! `glib::spawn_future_local` (lihat [`crate::progress`]).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;
use zippy_core::{ProgressEvent, ProgressSink};

use crate::file_list::{self, FileListView};
use crate::progress::ChannelSink;

/// Handle widget yang dibagi antar-callback selama window hidup.
struct Ui {
    window: adw::ApplicationWindow,
    toast: adw::ToastOverlay,
    list: FileListView,
    status: gtk::Label,
    revealer: gtk::Revealer,
    bar: gtk::ProgressBar,
    progress_label: gtk::Label,
    extract_btn: gtk::Button,
    /// Archive yang sedang dibuka (None bila belum ada).
    current: RefCell<Option<PathBuf>>,
}

pub fn build_ui(app: &adw::Application) {
    let header = adw::HeaderBar::new();

    let open_btn = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text("Buka archive")
        .build();
    let extract_btn = gtk::Button::builder()
        .label("Extract")
        .icon_name("extract-archive-symbolic")
        .tooltip_text("Extract semua isi")
        .sensitive(false)
        .build();
    let add_btn = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Buat archive baru dari berkas")
        .build();

    header.pack_start(&open_btn);
    header.pack_start(&extract_btn);
    header.pack_end(&add_btn);

    let list = file_list::build();

    let status = gtk::Label::builder()
        .label("Belum ada archive terbuka")
        .xalign(0.0)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    status.add_css_class("dim-label");

    // Progress: label + bar dalam revealer (tersembunyi saat idle).
    let bar = gtk::ProgressBar::builder().show_text(false).hexpand(true).build();
    let progress_label = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
    let progress_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    progress_box.set_margin_start(8);
    progress_box.set_margin_end(8);
    progress_box.set_margin_top(4);
    progress_box.set_margin_bottom(8);
    progress_box.append(&progress_label);
    progress_box.append(&bar);

    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(false)
        .child(&progress_box)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&list.widget);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&status);
    content.append(&revealer);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&toolbar));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Zippy")
        .default_width(900)
        .default_height(600)
        .content(&toast)
        .build();

    let ui = Rc::new(Ui {
        window: window.clone(),
        toast,
        list,
        status,
        revealer,
        bar,
        progress_label,
        extract_btn: extract_btn.clone(),
        current: RefCell::new(None),
    });

    open_btn.connect_clicked({
        let ui = ui.clone();
        move |_| open_dialog(&ui)
    });
    extract_btn.connect_clicked({
        let ui = ui.clone();
        move |_| extract_dialog(&ui)
    });
    add_btn.connect_clicked({
        let ui = ui.clone();
        move |_| compress_dialog(&ui)
    });

    window.present();

    // Dev-hook: ZIPPY_OPEN=<path> langsung membuka archive saat start. Berguna
    // untuk uji manual/headless jalur list tanpa harus klik dialog.
    if let Some(p) = std::env::var_os("ZIPPY_OPEN") {
        load_archive(&ui, PathBuf::from(p));
    }

    maybe_bench(app);
}

// ---------------------------------------------------------------------------
// Buka archive → list
// ---------------------------------------------------------------------------

fn open_dialog(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder().title("Buka Archive").build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.open(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(path) = file.path() {
                load_archive(&ui, path);
            }
        }
    });
}

fn load_archive(ui: &Rc<Ui>, path: PathBuf) {
    ui.status.set_text(&format!("Membaca {}…", path.display()));
    let (tx, rx) = async_channel::bounded(1);
    let worker_path = path.clone();
    std::thread::spawn(move || {
        let _ = tx.send_blocking(zippy_core::archive::list(&worker_path, None));
    });

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Ok(entries)) => {
                ui.list.store.remove_all();
                let total = entries.len();
                for e in &entries {
                    ui.list.store.append(&file_list::EntryObject::from_entry(e));
                }
                *ui.current.borrow_mut() = Some(path.clone());
                ui.extract_btn.set_sensitive(true);
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.status.set_text(&format!("{name} — {total} entri"));
                tracing::info!(entries = total, archive = %path.display(), "archive dibuka");
                // Dev-hook: ZIPPY_EXTRACT_TO=<dir> langsung meng-extract setelah
                // open — untuk uji jalur progress tanpa dialog.
                if let Some(dir) = std::env::var_os("ZIPPY_EXTRACT_TO") {
                    run_extract(&ui, path.clone(), PathBuf::from(dir));
                }
            }
            Ok(Err(e)) => {
                ui.status.set_text("Gagal membuka archive");
                show_toast(&ui, &format!("Gagal membuka: {e}"));
            }
            Err(_) => {} // worker hilang
        }
    });
}

// ---------------------------------------------------------------------------
// Extract semua
// ---------------------------------------------------------------------------

fn extract_dialog(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => {
            show_toast(ui, "Belum ada archive terbuka");
            return;
        }
    };

    let dialog = gtk::FileDialog::builder()
        .title("Extract ke folder…")
        .build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.select_folder(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(folder) = res {
            if let Some(dest) = folder.path() {
                run_extract(&ui, archive.clone(), dest);
            }
        }
    });
}

fn run_extract(ui: &Rc<Ui>, archive: PathBuf, dest: PathBuf) {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        let sink = ChannelSink::new(tx);
        if let Err(e) = zippy_core::archive::extract_all(&archive, &dest, None, &sink) {
            sink.emit(ProgressEvent::Error {
                message: e.to_string(),
            });
        }
        // sink drop → channel tertutup → loop UI berakhir.
    });

    ui.revealer.set_reveal_child(true);
    ui.bar.set_fraction(0.0);
    ui.progress_label.set_text("Memulai…");

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let mut total = 0usize;
        while let Ok(ev) = rx.recv().await {
            match ev {
                ProgressEvent::Started { total_files } => {
                    total = total_files;
                    ui.bar.set_fraction(0.0);
                }
                ProgressEvent::FileProcessed { name, index } => {
                    if total > 0 {
                        ui.bar.set_fraction((index + 1) as f64 / total as f64);
                    } else {
                        ui.bar.pulse();
                    }
                    ui.progress_label.set_text(&name);
                }
                ProgressEvent::BytesDone { bytes, total: t } => {
                    if t > 0 {
                        ui.bar.set_fraction(bytes as f64 / t as f64);
                    }
                }
                ProgressEvent::Finished { elapsed_ms } => {
                    ui.revealer.set_reveal_child(false);
                    show_toast(&ui, &format!("Extract selesai ({elapsed_ms} ms)"));
                    tracing::info!(elapsed_ms, "extract selesai");
                }
                ProgressEvent::Error { message } => {
                    ui.revealer.set_reveal_child(false);
                    show_toast(&ui, &format!("Gagal extract: {message}"));
                    tracing::error!(%message, "extract gagal");
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Compress (buat archive baru)
// ---------------------------------------------------------------------------

fn compress_dialog(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title("Pilih berkas/folder untuk diarsipkan")
        .build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.open_multiple(Some(&win), gio::Cancellable::NONE, move |res| {
        let Ok(files) = res else { return };
        let inputs = collect_paths(&files);
        if inputs.is_empty() {
            return;
        }
        choose_output(&ui, inputs);
    });
}

fn choose_output(ui: &Rc<Ui>, inputs: Vec<PathBuf>) {
    let save = gtk::FileDialog::builder()
        .title("Simpan archive sebagai…")
        .initial_name("archive.zip")
        .build();
    let win = ui.window.clone();
    let ui = ui.clone();
    save.save(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(dest) = file.path() {
                run_compress(&ui, inputs.clone(), dest);
            }
        }
    });
}

fn run_compress(ui: &Rc<Ui>, inputs: Vec<PathBuf>, dest: PathBuf) {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        let sink = ChannelSink::new(tx);
        let refs: Vec<&std::path::Path> = inputs.iter().map(|p| p.as_path()).collect();
        if let Err(e) = zippy_core::archive::compress(&refs, &dest, None, &sink) {
            sink.emit(ProgressEvent::Error {
                message: e.to_string(),
            });
        }
    });

    ui.revealer.set_reveal_child(true);
    ui.bar.set_fraction(0.0);
    ui.progress_label.set_text("Mengompres…");

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let mut total = 0usize;
        while let Ok(ev) = rx.recv().await {
            match ev {
                ProgressEvent::Started { total_files } => total = total_files,
                ProgressEvent::FileProcessed { name, index } => {
                    if total > 0 {
                        ui.bar.set_fraction((index + 1) as f64 / total as f64);
                    } else {
                        ui.bar.pulse();
                    }
                    ui.progress_label.set_text(&name);
                }
                ProgressEvent::BytesDone { .. } => {}
                ProgressEvent::Finished { elapsed_ms } => {
                    ui.revealer.set_reveal_child(false);
                    show_toast(&ui, &format!("Archive dibuat ({elapsed_ms} ms)"));
                }
                ProgressEvent::Error { message } => {
                    ui.revealer.set_reveal_child(false);
                    show_toast(&ui, &format!("Gagal kompres: {message}"));
                }
            }
        }
    });
}

/// Kumpulkan path dari hasil `open_multiple` (gio::ListModel of gio::File).
fn collect_paths(files: &gio::ListModel) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for i in 0..files.n_items() {
        if let Some(file) = files.item(i).and_downcast::<gio::File>() {
            if let Some(p) = file.path() {
                out.push(p);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// util
// ---------------------------------------------------------------------------

fn show_toast(ui: &Ui, msg: &str) {
    ui.toast.add_toast(adw::Toast::new(msg));
}

/// Mode benchmark Sprint 0 (dipakai scripts/measure.sh): bila `ZIPPY_BENCH`
/// diset, biarkan window settle lalu laporkan RSS dan quit.
fn maybe_bench(app: &adw::Application) {
    if std::env::var_os("ZIPPY_BENCH").is_none() {
        return;
    }
    let app = app.clone();
    glib::timeout_add_local_once(Duration::from_millis(800), move || {
        match read_vmrss_kb() {
            Some(kb) => println!("ZIPPY_BENCH rss_kb={kb} rss_mb={:.1}", kb as f64 / 1024.0),
            None => println!("ZIPPY_BENCH rss_kb=unknown"),
        }
        app.quit();
    });
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
