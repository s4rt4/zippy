//! Main window — gaya WinRAR (Planning Doc §5.1).
//!
//! Layout: menu bar → toolbar (ikon Papirus berlabel) → address bar →
//! GtkColumnView (Nama|Ukuran|Packed|Tipe|Modified|CRC32) dengan navigasi folder
//! (baris ".." + masuk sub-folder) → status bar. Operasi berat (list/extract/
//! compress) berjalan di `std::thread`; progress di-marshal ke UI lewat
//! `async-channel` + `glib::spawn_future_local` (lihat [`crate::progress`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;
use zippy_core::{ArchiveKind, CancelToken, Entry, Error, ProgressEvent, ProgressSink};

use crate::file_list::{self, FileListView, Row};
use crate::progress::ChannelSink;

/// Handle widget yang dibagi antar-callback selama window hidup.
struct Ui {
    window: gtk::ApplicationWindow,
    toast: adw::ToastOverlay,
    list: FileListView,
    address: gtk::Label,
    status: gtk::Label,
    revealer: gtk::Revealer,
    bar: gtk::ProgressBar,
    progress_label: gtk::Label,
    cancel_btn: gtk::Button,
    extract_btn: gtk::Button,
    search_bar: gtk::SearchBar,
    /// Filter nama aktif (dari Find); kosong = tampilkan semua.
    filter: RefCell<String>,
    /// Archive yang sedang dibuka (None bila belum ada).
    current: RefCell<Option<PathBuf>>,
    /// Token operasi yang sedang berjalan (None bila idle).
    cancel: RefCell<Option<CancelToken>>,
    /// Daftar entry mentah hasil `list` (sumber navigasi folder).
    entries: RefCell<Vec<Entry>>,
    /// Direktori yang sedang ditampilkan (komponen path; kosong = root).
    cwd: RefCell<Vec<String>>,
}

pub fn build_ui(app: &adw::Application) {
    let list = file_list::build();

    let address = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .hexpand(true)
        .margin_start(6)
        .margin_end(6)
        .margin_top(3)
        .margin_bottom(3)
        .build();
    address.add_css_class("dim-label");

    let status = gtk::Label::builder().xalign(0.0).hexpand(true).build();
    status.add_css_class("dim-label");

    // Progress (revealer di atas status bar).
    let bar = gtk::ProgressBar::builder()
        .show_text(false)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    let progress_label = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    let cancel_btn = gtk::Button::builder()
        .icon_name("process-stop")
        .tooltip_text("Batalkan operasi")
        .build();
    cancel_btn.add_css_class("flat");
    let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    progress_row.append(&bar);
    progress_row.append(&cancel_btn);
    let progress_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    progress_box.set_margin_start(8);
    progress_box.set_margin_end(8);
    progress_box.set_margin_top(4);
    progress_box.set_margin_bottom(6);
    progress_box.append(&progress_label);
    progress_box.append(&progress_row);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(false)
        .child(&progress_box)
        .build();

    // Toolbar: tombol Extract perlu referensi untuk enable/disable.
    let extract_btn = tool_button("archive-extract", "Extract To");

    // Search bar (Find): tersembunyi sampai diaktifkan tombol/menu Find.
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Filter berkas di folder ini…")
        .hexpand(true)
        .build();
    let search_bar = gtk::SearchBar::builder()
        .child(&search_entry)
        .key_capture_widget(&list.column_view)
        .build();
    search_bar.connect_entry(&search_entry);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Zippy")
        .icon_name("io.github.s4rt4.Zippy")
        .default_width(820)
        .default_height(540)
        .build();

    let ui = Rc::new(Ui {
        window: window.clone(),
        toast: adw::ToastOverlay::new(),
        list,
        address: address.clone(),
        status: status.clone(),
        revealer: revealer.clone(),
        bar,
        progress_label,
        cancel_btn: cancel_btn.clone(),
        extract_btn: extract_btn.clone(),
        search_bar: search_bar.clone(),
        filter: RefCell::new(String::new()),
        current: RefCell::new(None),
        cancel: RefCell::new(None),
        entries: RefCell::new(Vec::new()),
        cwd: RefCell::new(Vec::new()),
    });

    // --- Menu bar + aksi ---
    let menubar = build_menubar(&ui);

    // --- Toolbar ---
    let toolbar = build_toolbar(&ui, &extract_btn);

    // --- Address bar (dengan frame inset) ---
    let addr_icon = gtk::Image::from_icon_name("application-x-archive");
    let addr_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    addr_box.set_margin_start(6);
    addr_box.append(&addr_icon);
    addr_box.append(&address);
    let addr_frame = gtk::Frame::builder().child(&addr_box).build();
    addr_frame.set_margin_start(6);
    addr_frame.set_margin_end(6);
    addr_frame.set_margin_top(2);
    addr_frame.set_margin_bottom(2);

    // --- Status bar ---
    let statusbar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    statusbar.set_margin_start(8);
    statusbar.set_margin_end(8);
    statusbar.set_margin_top(2);
    statusbar.set_margin_bottom(2);
    statusbar.append(&status);

    // --- Susun ---
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&menubar);
    content.append(&toolbar);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&addr_frame);
    content.append(&search_bar);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&ui.list.widget);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&revealer);
    content.append(&statusbar);

    ui.toast.set_child(Some(&content));
    window.set_child(Some(&ui.toast));

    // Cancel button.
    cancel_btn.connect_clicked({
        let ui = ui.clone();
        move |btn| {
            if let Some(token) = ui.cancel.borrow().as_ref() {
                token.cancel();
            }
            btn.set_sensitive(false);
            ui.progress_label.set_text("Membatalkan…");
        }
    });

    // Find: perbarui filter saat teks pencarian berubah.
    search_entry.connect_search_changed({
        let ui = ui.clone();
        move |e| {
            *ui.filter.borrow_mut() = e.text().to_string();
            render(&ui);
        }
    });

    // Navigasi: double-click / Enter pada baris.
    ui.list.column_view.connect_activate({
        let ui = ui.clone();
        move |cv, pos| {
            let Some(obj) = cv
                .model()
                .and_then(|m| m.item(pos))
                .and_downcast::<file_list::EntryObject>()
            else {
                return;
            };
            if obj.is_parent() {
                ui.cwd.borrow_mut().pop();
                render(&ui);
            } else if obj.is_dir() {
                ui.cwd.borrow_mut().push(obj.name());
                render(&ui);
            } else {
                view_entry(&ui, &obj);
            }
        }
    });

    setup_drop_target(&ui);
    setup_context_menu(&ui);
    render(&ui);

    window.present();

    // Archive dari argumen CLI (path polos / --open / MIME handler).
    if let Some(p) = INITIAL_ARCHIVE.with(|c| c.borrow_mut().take()) {
        load_archive(&ui, p);
    } else if let Some(p) = std::env::var_os("ZIPPY_OPEN") {
        load_archive(&ui, PathBuf::from(p));
    }
    // "Add to archive…" dari file manager → langsung dialog buat-archive.
    if let Some(inputs) = INITIAL_COMPRESS.with(|c| c.borrow_mut().take()) {
        choose_output(&ui, inputs);
    }
    maybe_bench(app);
}

thread_local! {
    /// Archive yang diminta dibuka dari argumen CLI (di-set sebelum `app.run`).
    static INITIAL_ARCHIVE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Berkas yang akan dikompres (verb `--add`), di-set sebelum `app.run`.
    static INITIAL_COMPRESS: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
}

/// Set archive yang akan dibuka otomatis saat GUI start (dipanggil dari `main`).
pub fn set_initial_archive(path: PathBuf) {
    INITIAL_ARCHIVE.with(|c| *c.borrow_mut() = Some(path));
}

/// Set berkas yang akan dikompres lewat dialog saat GUI start.
pub fn set_initial_compress(inputs: Vec<PathBuf>) {
    INITIAL_COMPRESS.with(|c| *c.borrow_mut() = Some(inputs));
}

// ---------------------------------------------------------------------------
// Menu bar
// ---------------------------------------------------------------------------

fn build_menubar(ui: &Rc<Ui>) -> gtk::PopoverMenuBar {
    let group = gio::SimpleActionGroup::new();
    add_action(&group, "open", ui, open_dialog);
    add_action(&group, "close", ui, close_archive);
    add_action(&group, "quit", ui, |ui| ui.window.close());
    add_action(&group, "add", ui, compress_dialog);
    add_action(&group, "extract", ui, extract_dialog);
    add_action(&group, "test", ui, test_dialog);
    add_action(&group, "delete", ui, |ui| show_toast(ui, "Delete belum didukung"));
    add_action(&group, "find", ui, toggle_search);
    add_action(&group, "wizard", ui, |ui| show_toast(ui, "Wizard belum didukung"));
    add_action(&group, "about", ui, show_about);
    // Aksi context menu (beroperasi pada seleksi saat diaktifkan).
    add_action(&group, "view", ui, view_selected);
    add_action(&group, "up", ui, |ui| {
        ui.cwd.borrow_mut().pop();
        render(ui);
    });
    add_action(&group, "open_folder", ui, open_selected_folder);
    add_action(&group, "extract_sel", ui, extract_selected);
    add_action(&group, "copy_name", ui, copy_selected_names);
    add_action(&group, "props", ui, show_properties);
    ui.window.insert_action_group("win", Some(&group));

    let menu = gio::Menu::new();

    let file = gio::Menu::new();
    file.append(Some("Buka Archive…"), Some("win.open"));
    file.append(Some("Tutup Archive"), Some("win.close"));
    file.append(Some("Keluar"), Some("win.quit"));
    menu.append_submenu(Some("File"), &file);

    let cmds = gio::Menu::new();
    cmds.append(Some("Tambah Berkas…"), Some("win.add"));
    cmds.append(Some("Extract Ke…"), Some("win.extract"));
    cmds.append(Some("Test"), Some("win.test"));
    cmds.append(Some("Hapus"), Some("win.delete"));
    menu.append_submenu(Some("Commands"), &cmds);

    let tools = gio::Menu::new();
    tools.append(Some("Wizard"), Some("win.wizard"));
    tools.append(Some("Cari…"), Some("win.find"));
    menu.append_submenu(Some("Tools"), &tools);

    menu.append_submenu(Some("Favorites"), &gio::Menu::new());
    menu.append_submenu(Some("Options"), &gio::Menu::new());

    let help = gio::Menu::new();
    help.append(Some("Tentang Zippy"), Some("win.about"));
    menu.append_submenu(Some("Help"), &help);

    gtk::PopoverMenuBar::from_model(Some(&menu))
}

fn add_action<F: Fn(&Rc<Ui>) + 'static>(
    group: &gio::SimpleActionGroup,
    name: &str,
    ui: &Rc<Ui>,
    f: F,
) {
    let action = gio::SimpleAction::new(name, None);
    let ui = ui.clone();
    action.connect_activate(move |_, _| f(&ui));
    group.add_action(&action);
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

fn build_toolbar(ui: &Rc<Ui>, extract_btn: &gtk::Button) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.set_margin_start(4);
    bar.set_margin_end(4);
    bar.set_margin_top(4);
    bar.set_margin_bottom(4);

    let add = tool_button("add-files-to-archive", "Add");
    add.connect_clicked({
        let ui = ui.clone();
        move |_| compress_dialog(&ui)
    });
    extract_btn.connect_clicked({
        let ui = ui.clone();
        move |_| extract_dialog(&ui)
    });
    extract_btn.set_sensitive(false);

    let test = tool_button("dialog-ok-apply", "Test");
    test.connect_clicked({
        let ui = ui.clone();
        move |_| test_dialog(&ui)
    });
    let view = tool_button("document-preview-archive", "View");
    view.connect_clicked({
        let ui = ui.clone();
        move |_| view_selected(&ui)
    });
    let delete = tool_button("archive-remove", "Delete");
    delete.connect_clicked({
        let ui = ui.clone();
        move |_| show_toast(&ui, "Delete belum didukung")
    });
    let find = tool_button("edit-find", "Find");
    find.connect_clicked({
        let ui = ui.clone();
        move |_| toggle_search(&ui)
    });
    let wizard = tool_button("tools-wizard", "Wizard");
    wizard.connect_clicked({
        let ui = ui.clone();
        move |_| show_toast(&ui, "Wizard belum didukung")
    });
    let info = tool_button("dialog-information", "Info");
    info.connect_clicked({
        let ui = ui.clone();
        move |_| show_about(&ui)
    });

    bar.append(&add);
    bar.append(extract_btn);
    bar.append(&test);
    bar.append(&view);
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&delete);
    bar.append(&find);
    bar.append(&wizard);
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&info);
    bar
}

/// Tombol toolbar gaya WinRAR: ikon besar di atas, label di bawah.
fn tool_button(icon: &str, label: &str) -> gtk::Button {
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(32);
    let lbl = gtk::Label::new(Some(label));
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.append(&image);
    b.append(&lbl);
    let btn = gtk::Button::builder().child(&b).tooltip_text(label).build();
    btn.add_css_class("flat");
    btn
}

// ---------------------------------------------------------------------------
// Render daftar (navigasi folder)
// ---------------------------------------------------------------------------

/// Hitung baris untuk direktori `cwd`: sub-folder langsung (eksplisit maupun
/// implisit dari path bersarang) lalu berkas langsung.
fn rows_for_dir(entries: &[Entry], cwd: &[String]) -> Vec<Row> {
    let prefix = if cwd.is_empty() {
        String::new()
    } else {
        format!("{}/", cwd.join("/"))
    };

    let mut dirs: HashMap<String, Row> = HashMap::new();
    let mut files: Vec<Row> = Vec::new();

    for e in entries {
        let raw = e.name.trim_end_matches('/');
        if raw.is_empty() || !raw.starts_with(&prefix) {
            continue;
        }
        let rest = &raw[prefix.len()..];
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('/') {
            // Anak langsung.
            None => {
                if e.is_dir {
                    dirs.entry(rest.to_string()).or_insert_with(|| Row {
                        name: rest.to_string(),
                        full_path: raw.to_string(),
                        is_dir: true,
                        is_parent: false,
                        size: 0,
                        packed: 0,
                        modified: e.modified.clone().unwrap_or_default(),
                        crc: None,
                    });
                } else {
                    files.push(Row {
                        name: rest.to_string(),
                        full_path: raw.to_string(),
                        is_dir: false,
                        is_parent: false,
                        size: e.size,
                        packed: e.compressed_size,
                        modified: e.modified.clone().unwrap_or_default(),
                        crc: e.crc32,
                    });
                }
            }
            // Bersarang → menyiratkan sub-folder `first`.
            Some((first, _)) => {
                dirs.entry(first.to_string()).or_insert_with(|| Row {
                    name: first.to_string(),
                    full_path: format!("{prefix}{first}"),
                    is_dir: true,
                    is_parent: false,
                    size: 0,
                    packed: 0,
                    modified: String::new(),
                    crc: None,
                });
            }
        }
    }

    let mut dir_rows: Vec<Row> = dirs.into_values().collect();
    dir_rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    dir_rows.extend(files);
    dir_rows
}

/// Bangun ulang daftar untuk `cwd` saat ini + perbarui address & status bar.
fn render(ui: &Rc<Ui>) {
    let entries = ui.entries.borrow();
    let cwd = ui.cwd.borrow();
    ui.list.store.remove_all();

    // Baris ".." untuk naik (kecuali di root).
    if !cwd.is_empty() {
        ui.list.store.append(&file_list::EntryObject::from_row(&Row {
            name: "..".to_string(),
            full_path: String::new(),
            is_dir: true,
            is_parent: true,
            size: 0,
            packed: 0,
            modified: String::new(),
            crc: None,
        }));
    }

    let filter = ui.filter.borrow().to_lowercase();
    let rows = rows_for_dir(&entries, &cwd);
    let (mut folders, mut files, mut bytes) = (0u64, 0u64, 0u64);
    for r in &rows {
        if !filter.is_empty() && !r.name.to_lowercase().contains(&filter) {
            continue;
        }
        if r.is_dir {
            folders += 1;
        } else {
            files += 1;
            bytes += r.size;
        }
        ui.list.store.append(&file_list::EntryObject::from_row(r));
    }

    ui.status.set_text(&format!(
        "Total {folders} folder dan {} bita dalam {files} berkas",
        file_list::group_thousands(bytes)
    ));
    update_address(ui, &entries, &cwd);
}

fn update_address(ui: &Rc<Ui>, entries: &[Entry], cwd: &[String]) {
    let current = ui.current.borrow();
    let Some(path) = current.as_ref() else {
        ui.address.set_text("Tidak ada archive terbuka");
        return;
    };
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shown = if cwd.is_empty() {
        name
    } else {
        format!("{name}\\{}", cwd.join("\\"))
    };
    let kind = kind_label(path);
    let total: u64 = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
    ui.address.set_text(&format!(
        "{shown} - {kind} archive, unpacked size {} bytes",
        file_list::group_thousands(total)
    ));
}

/// Label format singkat (ZIP / TAR.GZ / 7Z …) dari ekstensi.
fn kind_label(path: &Path) -> String {
    match zippy_core::archive::kind_from_ext(path) {
        Some(ArchiveKind::Zip) => "ZIP",
        Some(ArchiveKind::Tar) => "TAR",
        Some(ArchiveKind::TarGz) => "TAR.GZ",
        Some(ArchiveKind::TarBz2) => "TAR.BZ2",
        Some(ArchiveKind::TarXz) => "TAR.XZ",
        Some(ArchiveKind::TarZst) => "TAR.ZST",
        Some(ArchiveKind::Gz) => "GZIP",
        Some(ArchiveKind::Bz2) => "BZIP2",
        Some(ArchiveKind::Xz) => "XZ",
        Some(ArchiveKind::Zst) => "ZSTD",
        Some(ArchiveKind::SevenZip) => "7Z",
        Some(ArchiveKind::Rar) => "RAR",
        None => "",
    }
    .to_string()
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
                let total = entries.len();
                *ui.entries.borrow_mut() = entries;
                ui.cwd.borrow_mut().clear();
                *ui.current.borrow_mut() = Some(path.clone());
                ui.extract_btn.set_sensitive(true);
                render(&ui);
                tracing::info!(entries = total, archive = %path.display(), "archive dibuka");
                if let Some(dir) = std::env::var_os("ZIPPY_EXTRACT_TO") {
                    let pw = std::env::var("ZIPPY_PASSWORD").ok();
                    run_extract(&ui, path.clone(), PathBuf::from(dir), pw);
                }
                if std::env::var_os("ZIPPY_TEST").is_some() {
                    run_test(&ui, path.clone(), std::env::var("ZIPPY_PASSWORD").ok());
                }
            }
            Ok(Err(e)) => {
                ui.status.set_text("Gagal membuka archive");
                show_toast(&ui, &format!("Gagal membuka: {e}"));
            }
            Err(_) => {}
        }
    });
}

/// Tutup archive: kosongkan daftar & reset state.
fn close_archive(ui: &Rc<Ui>) {
    ui.entries.borrow_mut().clear();
    ui.cwd.borrow_mut().clear();
    *ui.current.borrow_mut() = None;
    ui.extract_btn.set_sensitive(false);
    render(ui);
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

    let dialog = gtk::FileDialog::builder().title("Extract ke folder…").build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.select_folder(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(folder) = res {
            if let Some(dest) = folder.path() {
                run_extract(&ui, archive.clone(), dest, None);
            }
        }
    });
}

/// Extract `archive` → `dest`. `password` dipakai bila archive terenkripsi;
/// bila `None` dan core melaporkan [`Error::Password`], UI memunculkan dialog
/// password lalu memanggil ulang dengan password yang dimasukkan.
fn run_extract(ui: &Rc<Ui>, archive: PathBuf, dest: PathBuf, password: Option<String>) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_ev, rx_ev) = async_channel::unbounded();
    let (tx_done, rx_done) = async_channel::bounded(1);
    let worker_archive = archive.clone();
    let worker_dest = dest.clone();
    let worker_pw = password.clone();
    std::thread::spawn(move || {
        let res = {
            let sink = ChannelSink::new(tx_ev);
            zippy_core::archive::extract_all(
                &worker_archive,
                &worker_dest,
                worker_pw.as_deref(),
                &cancel,
                &sink,
            )
        };
        let _ = tx_done.send_blocking(res);
    });

    ui.revealer.set_reveal_child(true);
    ui.cancel_btn.set_sensitive(true);
    ui.bar.set_fraction(0.0);
    ui.progress_label.set_text("Memulai…");
    schedule_dev_cancel(ui);

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let mut total = 0usize;
        while let Ok(ev) = rx_ev.recv().await {
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
                ProgressEvent::Finished { .. } | ProgressEvent::Error { .. } => {}
            }
        }

        ui.revealer.set_reveal_child(false);
        *ui.cancel.borrow_mut() = None;
        match rx_done.recv().await {
            Ok(Ok(())) => {
                show_toast(&ui, "Extract selesai");
                tracing::info!("extract selesai");
            }
            Ok(Err(Error::Cancelled)) => {
                show_toast(&ui, "Extract dibatalkan");
                tracing::info!("extract dibatalkan");
            }
            Ok(Err(Error::Password)) => {
                tracing::warn!("extract perlu password");
                prompt_password(&ui, &archive, &dest);
            }
            Ok(Err(e)) => {
                show_toast(&ui, &format!("Gagal extract: {e}"));
                tracing::error!(error = %e, "extract gagal");
            }
            Err(_) => {}
        }
    });
}

// ---------------------------------------------------------------------------
// Dialog password
// ---------------------------------------------------------------------------

/// Dialog password generik. `on_ok` dipanggil dengan `Some(pw)` bila diisi atau
/// `None` bila kosong (caller yang memutuskan arti kosong).
fn ask_password<F>(ui: &Rc<Ui>, heading: &str, body: &str, ok_label: &str, on_ok: F)
where
    F: Fn(&Rc<Ui>, Option<String>) + 'static,
{
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .activates_default(true)
        .build();
    let dialog = adw::MessageDialog::new(Some(&ui.window), Some(heading), Some(body));
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Batal");
    dialog.add_response("ok", ok_label);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let ui = ui.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "ok" {
            return;
        }
        let pw = entry.text().to_string();
        on_ok(&ui, if pw.is_empty() { None } else { Some(pw) });
    });
    dialog.present();
}

/// Minta password (wajib) untuk meng-extract archive terenkripsi, lalu retry.
fn prompt_password(ui: &Rc<Ui>, archive: &Path, dest: &Path) {
    let name = archive
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let archive = archive.to_path_buf();
    let dest = dest.to_path_buf();
    ask_password(
        ui,
        "Archive Terenkripsi",
        &format!("Masukkan password untuk \"{name}\"."),
        "Buka",
        move |ui, pw| match pw {
            Some(pw) => run_extract(ui, archive.clone(), dest.clone(), Some(pw)),
            None => show_toast(ui, "Password kosong"),
        },
    );
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
                compress_to(&ui, inputs.clone(), dest);
            }
        }
    });
}

/// Lanjut ke compress; bila format `dest` mendukung enkripsi (zip/7z), tawarkan
/// dialog password lebih dulu (kosong = tanpa enkripsi).
fn compress_to(ui: &Rc<Ui>, inputs: Vec<PathBuf>, dest: PathBuf) {
    let supports_pw = matches!(
        zippy_core::archive::kind_from_ext(&dest),
        Some(ArchiveKind::Zip | ArchiveKind::SevenZip)
    );
    if !supports_pw {
        run_compress(ui, inputs, dest, None);
        return;
    }
    ask_password(
        ui,
        "Lindungi Archive",
        "Masukkan password untuk enkripsi AES-256 (kosongkan untuk tanpa password).",
        "Buat",
        move |ui, pw| run_compress(ui, inputs.clone(), dest.clone(), pw),
    );
}

fn run_compress(ui: &Rc<Ui>, inputs: Vec<PathBuf>, dest: PathBuf, password: Option<String>) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_ev, rx_ev) = async_channel::unbounded();
    let (tx_done, rx_done) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let res = {
            let sink = ChannelSink::new(tx_ev);
            let refs: Vec<&Path> = inputs.iter().map(|p| p.as_path()).collect();
            zippy_core::archive::compress(&refs, &dest, password.as_deref(), &cancel, &sink)
        };
        let _ = tx_done.send_blocking(res);
    });

    ui.revealer.set_reveal_child(true);
    ui.cancel_btn.set_sensitive(true);
    ui.bar.set_fraction(0.0);
    ui.progress_label.set_text("Mengompres…");

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let mut total = 0usize;
        while let Ok(ev) = rx_ev.recv().await {
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
                ProgressEvent::BytesDone { .. }
                | ProgressEvent::Finished { .. }
                | ProgressEvent::Error { .. } => {}
            }
        }

        ui.revealer.set_reveal_child(false);
        *ui.cancel.borrow_mut() = None;
        match rx_done.recv().await {
            Ok(Ok(())) => show_toast(&ui, "Archive dibuat"),
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Kompres dibatalkan"),
            Ok(Err(e)) => show_toast(&ui, &format!("Gagal kompres: {e}")),
            Err(_) => {}
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

/// Dev-hook: `ZIPPY_CANCEL_MS=<ms>` → batalkan operasi berjalan setelah `ms`.
fn schedule_dev_cancel(ui: &Rc<Ui>) {
    let Some(ms) = std::env::var("ZIPPY_CANCEL_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    else {
        return;
    };
    let ui = ui.clone();
    glib::timeout_add_local_once(Duration::from_millis(ms), move || {
        if let Some(token) = ui.cancel.borrow().as_ref() {
            token.cancel();
        }
    });
}

// ---------------------------------------------------------------------------
// Test (verifikasi integritas)
// ---------------------------------------------------------------------------

fn test_dialog(ui: &Rc<Ui>) {
    match ui.current.borrow().clone() {
        Some(archive) => run_test(ui, archive, None),
        None => show_toast(ui, "Belum ada archive terbuka"),
    }
}

fn run_test(ui: &Rc<Ui>, archive: PathBuf, password: Option<String>) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_ev, rx_ev) = async_channel::unbounded();
    let (tx_done, rx_done) = async_channel::bounded(1);
    let worker_archive = archive.clone();
    let worker_pw = password.clone();
    std::thread::spawn(move || {
        let res = {
            let sink = ChannelSink::new(tx_ev);
            zippy_core::archive::test(&worker_archive, worker_pw.as_deref(), &cancel, &sink)
        };
        let _ = tx_done.send_blocking(res);
    });

    ui.revealer.set_reveal_child(true);
    ui.cancel_btn.set_sensitive(true);
    ui.bar.set_fraction(0.0);
    ui.progress_label.set_text("Menguji…");
    schedule_dev_cancel(ui);

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let mut total = 0usize;
        while let Ok(ev) = rx_ev.recv().await {
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
                _ => {}
            }
        }

        ui.revealer.set_reveal_child(false);
        *ui.cancel.borrow_mut() = None;
        match rx_done.recv().await {
            Ok(Ok(())) => {
                show_toast(&ui, "Test selesai — tidak ada kesalahan ditemukan");
                tracing::info!("test ok");
            }
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Test dibatalkan"),
            Ok(Err(Error::Password)) => {
                let archive = archive.clone();
                ask_password(
                    &ui,
                    "Archive Terenkripsi",
                    "Masukkan password untuk menguji.",
                    "Uji",
                    move |ui, pw| match pw {
                        Some(pw) => run_test(ui, archive.clone(), Some(pw)),
                        None => show_toast(ui, "Password kosong"),
                    },
                );
            }
            Ok(Err(e)) => show_toast(&ui, &format!("Test GAGAL: {e}")),
            Err(_) => {}
        }
    });
}

// ---------------------------------------------------------------------------
// View (buka satu berkas)
// ---------------------------------------------------------------------------

fn view_selected(ui: &Rc<Ui>) {
    match selected_entry(ui) {
        Some(obj) => view_entry(ui, &obj),
        None => show_toast(ui, "Pilih berkas dulu"),
    }
}

fn view_entry(ui: &Rc<Ui>, obj: &file_list::EntryObject) {
    if obj.is_parent() || obj.is_dir() {
        show_toast(ui, "Pilih berkas, bukan folder");
        return;
    }
    match ui.current.borrow().clone() {
        Some(archive) => run_view(ui, archive, obj.full_path(), None),
        None => {}
    }
}

fn run_view(ui: &Rc<Ui>, archive: PathBuf, name: String, password: Option<String>) {
    let dest = std::env::temp_dir().join("zippy-view");
    let cancel = CancelToken::new();
    let (tx, rx) = async_channel::bounded(1);
    let worker_archive = archive.clone();
    let worker_name = name.clone();
    let worker_pw = password.clone();
    let worker_dest = dest.clone();
    std::thread::spawn(move || {
        let res = zippy_core::archive::extract_entry(
            &worker_archive,
            &worker_name,
            &worker_dest,
            worker_pw.as_deref(),
            &cancel,
        );
        let _ = tx.send_blocking(res);
    });

    ui.progress_label.set_text(&format!("Membuka {name}…"));
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Ok(path)) => launch_file(&ui, &path),
            Ok(Err(Error::Password)) => {
                let archive = archive.clone();
                let name = name.clone();
                ask_password(
                    &ui,
                    "Archive Terenkripsi",
                    "Masukkan password untuk membuka berkas.",
                    "Buka",
                    move |ui, pw| match pw {
                        Some(pw) => run_view(ui, archive.clone(), name.clone(), Some(pw)),
                        None => show_toast(ui, "Password kosong"),
                    },
                );
            }
            Ok(Err(e)) => show_toast(&ui, &format!("Gagal membuka: {e}")),
            Err(_) => {}
        }
    });
}

fn launch_file(ui: &Rc<Ui>, path: &Path) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.launch(Some(&ui.window), gio::Cancellable::NONE, |res| {
        if let Err(e) = res {
            tracing::warn!("launch berkas gagal: {e}");
        }
    });
}

/// Entry yang sedang dipilih di daftar (yang pertama bila multi-select).
fn selected_entry(ui: &Rc<Ui>) -> Option<file_list::EntryObject> {
    let model = ui.list.column_view.model()?;
    for i in 0..model.n_items() {
        if model.is_selected(i) {
            return model.item(i).and_downcast::<file_list::EntryObject>();
        }
    }
    None
}

/// Toggle search bar (Find). Saat ditutup, bersihkan filter.
fn toggle_search(ui: &Rc<Ui>) {
    let on = !ui.search_bar.is_search_mode();
    ui.search_bar.set_search_mode(on);
    if !on {
        ui.filter.borrow_mut().clear();
        render(ui);
    }
}

// ---------------------------------------------------------------------------
// Drag-and-drop & context menu
// ---------------------------------------------------------------------------

fn setup_drop_target(ui: &Rc<Ui>) {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    target.connect_drop({
        let ui = ui.clone();
        move |_, value, _, _| {
            let Ok(list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let paths: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
            if paths.is_empty() {
                return false;
            }
            handle_drop(&ui, paths);
            true
        }
    });
    ui.list.widget.add_controller(target);
}

fn handle_drop(ui: &Rc<Ui>, paths: Vec<PathBuf>) {
    let single_archive = paths.len() == 1
        && paths[0].is_file()
        && zippy_core::archive::kind_from_ext(&paths[0]).is_some();
    if single_archive {
        load_archive(ui, paths.into_iter().next().unwrap());
    } else {
        choose_output(ui, paths);
    }
}

/// Menu klik-kanan **kondisional** pada daftar isi — pembeda Zippy (Planning
/// Doc §4). Isi menu berubah sesuai seleksi: berkas, folder, "..", banyak item,
/// atau area kosong.
fn setup_context_menu(ui: &Rc<Ui>) {
    let popover = gtk::PopoverMenu::from_model(gio::MenuModel::NONE);
    popover.set_parent(&ui.list.widget);
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);

    let gesture = gtk::GestureClick::builder()
        .button(gdk::BUTTON_SECONDARY)
        .build();
    gesture.connect_pressed({
        let ui = ui.clone();
        move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let menu = build_context_menu(&ui);
            popover.set_menu_model(Some(&menu));
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        }
    });
    ui.list.widget.add_controller(gesture);
}

/// Susun model menu sesuai seleksi saat ini.
fn build_context_menu(ui: &Rc<Ui>) -> gio::Menu {
    let sel = selected_entries(ui);
    let menu = gio::Menu::new();

    match sel.as_slice() {
        // Satu baris terpilih.
        [o] if o.is_parent() => {
            menu.append(Some("Naik ke folder induk"), Some("win.up"));
        }
        [o] if o.is_dir() => {
            let s = gio::Menu::new();
            s.append(Some("Buka folder"), Some("win.open_folder"));
            s.append(Some("Extract folder ini…"), Some("win.extract_sel"));
            menu.append_section(None, &s);
            let s2 = gio::Menu::new();
            s2.append(Some("Salin nama"), Some("win.copy_name"));
            s2.append(Some("Properti…"), Some("win.props"));
            menu.append_section(None, &s2);
        }
        [_o] => {
            let s = gio::Menu::new();
            s.append(Some("Buka (View)"), Some("win.view"));
            s.append(Some("Extract berkas ini…"), Some("win.extract_sel"));
            menu.append_section(None, &s);
            let s2 = gio::Menu::new();
            s2.append(Some("Salin nama"), Some("win.copy_name"));
            s2.append(Some("Properti…"), Some("win.props"));
            menu.append_section(None, &s2);
        }
        // Banyak baris terpilih.
        many if many.len() > 1 => {
            let s = gio::Menu::new();
            s.append(
                Some(&format!("Extract {} item terpilih…", many.len())),
                Some("win.extract_sel"),
            );
            s.append(Some("Salin nama"), Some("win.copy_name"));
            menu.append_section(None, &s);
        }
        _ => {}
    }

    // Aksi tingkat-archive (selalu ada bila archive terbuka).
    if ui.current.borrow().is_some() {
        let s = gio::Menu::new();
        s.append(Some("Extract Semua…"), Some("win.extract"));
        s.append(Some("Test Archive"), Some("win.test"));
        menu.append_section(None, &s);
        let s2 = gio::Menu::new();
        s2.append(Some("Tutup Archive"), Some("win.close"));
        menu.append_section(None, &s2);
    }
    menu
}

/// Semua entry yang sedang terpilih di daftar.
fn selected_entries(ui: &Rc<Ui>) -> Vec<file_list::EntryObject> {
    let mut out = Vec::new();
    if let Some(model) = ui.list.column_view.model() {
        for i in 0..model.n_items() {
            if model.is_selected(i) {
                if let Some(o) = model.item(i).and_downcast::<file_list::EntryObject>() {
                    out.push(o);
                }
            }
        }
    }
    out
}

/// Masuk ke folder yang terpilih (bila tepat satu folder).
fn open_selected_folder(ui: &Rc<Ui>) {
    if let [o] = selected_entries(ui).as_slice() {
        if o.is_dir() && !o.is_parent() {
            ui.cwd.borrow_mut().push(o.name());
            render(ui);
        }
    }
}

/// Salin nama entry terpilih ke clipboard (dipisah baris).
fn copy_selected_names(ui: &Rc<Ui>) {
    let names: Vec<String> = selected_entries(ui)
        .iter()
        .filter(|o| !o.is_parent())
        .map(|o| o.name())
        .collect();
    if names.is_empty() {
        return;
    }
    ui.window.clipboard().set_text(&names.join("\n"));
    show_toast(ui, &format!("{} nama disalin", names.len()));
}

/// Dialog properti untuk satu entry terpilih.
fn show_properties(ui: &Rc<Ui>) {
    let sel = selected_entries(ui);
    let [o] = sel.as_slice() else {
        return;
    };
    if o.is_parent() {
        return;
    }
    let ratio = if o.size() > 0 {
        format!("{:.1}%", o.packed() as f64 / o.size() as f64 * 100.0)
    } else {
        "—".to_string()
    };
    let body = format!(
        "Nama: {}\nPath: {}\nTipe: {}\nUkuran: {} bita\nPacked: {} bita\nRasio: {}\nModified: {}\nCRC32: {}",
        o.name(),
        o.full_path(),
        if o.is_dir() { "Folder" } else { "Berkas" },
        file_list::group_thousands(o.size()),
        file_list::group_thousands(o.packed()),
        ratio,
        opt_dash(o.modified()),
        opt_dash(o.crc_hex()),
    );
    let dialog = adw::MessageDialog::new(Some(&ui.window), Some("Properti"), Some(&body));
    dialog.add_response("ok", "Tutup");
    dialog.present();
}

fn opt_dash(s: String) -> String {
    if s.is_empty() {
        "—".to_string()
    } else {
        s
    }
}

/// Extract entry-entry terpilih (berkas, atau seluruh isi folder terpilih) ke
/// folder pilihan user — mempertahankan struktur path.
fn extract_selected(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => return,
    };
    let sel = selected_entries(ui);
    if sel.is_empty() {
        return;
    }

    // Perluas seleksi → daftar nama berkas (folder dijabarkan ke isinya).
    let entries = ui.entries.borrow();
    let mut names: Vec<String> = Vec::new();
    for o in &sel {
        if o.is_parent() {
            continue;
        }
        if o.is_dir() {
            let prefix = format!("{}/", o.full_path());
            for e in entries.iter() {
                if !e.is_dir && e.name.trim_end_matches('/').starts_with(&prefix) {
                    names.push(e.name.clone());
                }
            }
        } else {
            names.push(o.full_path());
        }
    }
    drop(entries);
    names.sort();
    names.dedup();
    if names.is_empty() {
        show_toast(ui, "Tidak ada berkas untuk di-extract");
        return;
    }

    let dialog = gtk::FileDialog::builder().title("Extract ke folder…").build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.select_folder(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(folder) = res {
            if let Some(dest) = folder.path() {
                run_extract_selected(&ui, archive.clone(), names.clone(), dest, None);
            }
        }
    });
}

fn run_extract_selected(
    ui: &Rc<Ui>,
    archive: PathBuf,
    names: Vec<String>,
    dest: PathBuf,
    password: Option<String>,
) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_ev, rx_ev) = async_channel::unbounded();
    let (tx_done, rx_done) = async_channel::bounded(1);
    let worker_archive = archive.clone();
    let worker_names = names.clone();
    let worker_dest = dest.clone();
    let worker_pw = password.clone();
    std::thread::spawn(move || {
        let sink = ChannelSink::new(tx_ev);
        let total = worker_names.len();
        sink.emit(ProgressEvent::Started { total_files: total });
        let mut res = Ok(());
        for (i, name) in worker_names.iter().enumerate() {
            if let Err(e) = cancel.check() {
                res = Err(e);
                break;
            }
            if let Err(e) = zippy_core::archive::extract_entry(
                &worker_archive,
                name,
                &worker_dest,
                worker_pw.as_deref(),
                &cancel,
            ) {
                res = Err(e);
                break;
            }
            sink.emit(ProgressEvent::FileProcessed {
                name: name.clone(),
                index: i,
            });
        }
        drop(sink);
        let _ = tx_done.send_blocking(res);
    });

    ui.revealer.set_reveal_child(true);
    ui.cancel_btn.set_sensitive(true);
    ui.bar.set_fraction(0.0);
    ui.progress_label.set_text("Memulai…");
    schedule_dev_cancel(ui);

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let mut total = 0usize;
        while let Ok(ev) = rx_ev.recv().await {
            match ev {
                ProgressEvent::Started { total_files } => total = total_files,
                ProgressEvent::FileProcessed { name, index } => {
                    if total > 0 {
                        ui.bar.set_fraction((index + 1) as f64 / total as f64);
                    }
                    ui.progress_label.set_text(&name);
                }
                _ => {}
            }
        }
        ui.revealer.set_reveal_child(false);
        *ui.cancel.borrow_mut() = None;
        match rx_done.recv().await {
            Ok(Ok(())) => show_toast(&ui, "Extract selesai"),
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Extract dibatalkan"),
            Ok(Err(Error::Password)) => {
                let archive = archive.clone();
                let names = names.clone();
                let dest = dest.clone();
                ask_password(
                    &ui,
                    "Archive Terenkripsi",
                    "Masukkan password untuk extract.",
                    "Extract",
                    move |ui, pw| match pw {
                        Some(pw) => {
                            run_extract_selected(ui, archive.clone(), names.clone(), dest.clone(), Some(pw))
                        }
                        None => show_toast(ui, "Password kosong"),
                    },
                );
            }
            Ok(Err(e)) => show_toast(&ui, &format!("Gagal extract: {e}")),
            Err(_) => {}
        }
    });
}

// ---------------------------------------------------------------------------
// util
// ---------------------------------------------------------------------------

fn show_about(ui: &Rc<Ui>) {
    let dialog = adw::MessageDialog::new(
        Some(&ui.window),
        Some("Zippy"),
        Some(&format!(
            "Archive manager untuk Linux — versi {}\n\nGTK4 + libadwaita, core Rust.",
            zippy_core::VERSION
        )),
    );
    dialog.add_response("ok", "Tutup");
    dialog.present();
}

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
