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
use zippy_core::{
    ArchiveKind, CancelToken, Entry, Error, Level, OverwriteMode, ProgressEvent, ProgressSink,
};

use crate::config::{self, Config};
use crate::file_list::{self, FileListView, Row};
use crate::progress::ChannelSink;

/// Nama ikon aplikasi (themed). Lihat [`setup_icon_theme`].
const APP_ICON: &str = "io.github.s4rt4.Zippy";
/// Ikon di-embed agar logo tetap muncul walau app belum di-install.
const APP_ICON_SVG: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../data/icons/io.github.s4rt4.Zippy.svg"));

/// Ikon aksi toolbar (gaya WinRAR berwarna) yang di-embed + ditulis ke icon
/// theme cache oleh [`setup_icon_theme`]. Tuple `(nama-themed, isi-svg)`.
macro_rules! action_icon {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../data/icons/actions/",
                $name,
                ".svg"
            )) as &[u8],
        )
    };
}
const ACTION_ICONS: &[(&str, &[u8])] = &[
    action_icon!("zippy-add"),
    action_icon!("zippy-extract"),
    action_icon!("zippy-test"),
    action_icon!("zippy-view"),
    action_icon!("zippy-delete"),
    action_icon!("zippy-find"),
    action_icon!("zippy-wizard"),
    action_icon!("zippy-info"),
    action_icon!("zippy-repair"),
    action_icon!("zippy-scan"),
    action_icon!("zippy-good"),
    action_icon!("zippy-bad"),
];

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
    /// Password default sesi (File → Set default password). Dipakai sebagai
    /// fallback saat operasi tidak diberi password eksplisit.
    default_password: RefCell<Option<String>>,
    /// Log operasi sesi (Options → Lihat Log). Tiap baris sudah ber-timestamp.
    log: RefCell<Vec<String>>,
    /// Token operasi yang sedang berjalan (None bila idle).
    cancel: RefCell<Option<CancelToken>>,
    /// Daftar entry mentah hasil `list` (sumber navigasi folder).
    entries: RefCell<Vec<Entry>>,
    /// Direktori yang sedang ditampilkan (komponen path; kosong = root).
    cwd: RefCell<Vec<String>>,
    /// Preferensi persisten (level default, tema, konfirmasi hapus).
    config: RefCell<Config>,
    /// Submenu Favorites — dibangun ulang saat daftar favorit berubah.
    favorites_menu: gio::Menu,
}

pub fn build_ui(app: &adw::Application) {
    // Daftarkan ikon embedded + terapkan tema sebelum membangun widget.
    setup_icon_theme();
    let cfg = Config::load();
    apply_scheme(cfg.scheme);

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
    let extract_btn = tool_button("zippy-extract", "Extract To");

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
        default_password: RefCell::new(None),
        log: RefCell::new(Vec::new()),
        cancel: RefCell::new(None),
        entries: RefCell::new(Vec::new()),
        cwd: RefCell::new(Vec::new()),
        config: RefCell::new(cfg),
        favorites_menu: gio::Menu::new(),
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

    // Akselerator keyboard gaya WinRAR.
    app.set_accels_for_action("win.rename", &["F2"]);
    app.set_accels_for_action("win.find", &["F3"]);
    app.set_accels_for_action("win.delete", &["Delete"]);
    app.set_accels_for_action("win.open", &["<Ctrl>o"]);

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
    add_action(&group, "set_pw", ui, set_default_password);
    add_action(&group, "save_copy", ui, save_archive_copy);
    add_action(&group, "select_all", ui, select_all_rows);
    add_action(&group, "invert_sel", ui, invert_selection);
    add_action(&group, "report", ui, generate_report);
    add_action(&group, "log", ui, show_log);
    add_action(&group, "add", ui, compress_dialog);
    add_action(&group, "extract", ui, extract_dialog);
    add_action(&group, "test", ui, test_dialog);
    add_action(&group, "delete", ui, delete_selected);
    add_action(&group, "rename", ui, rename_selected);
    add_action(&group, "find", ui, toggle_search);
    add_action(&group, "wizard", ui, show_wizard);
    add_action(&group, "repair", ui, repair_dialog);
    add_action(&group, "scan", ui, scan_dialog);
    add_action(&group, "convert", ui, convert_dialog);
    add_action(&group, "sfx", ui, sfx_dialog);
    add_action(&group, "comment", ui, comment_dialog);
    add_action(&group, "options", ui, show_preferences);
    add_action(&group, "about", ui, show_about);
    // Favorit: tambah/hapus arsip saat ini, kelola, dan buka (berparameter).
    add_action(&group, "fav_add", ui, fav_add_current);
    add_action(&group, "fav_remove", ui, fav_remove_current);
    add_action(&group, "fav_manage", ui, show_favorites_manager);
    let fav_open = gio::SimpleAction::new("fav_open", Some(glib::VariantTy::STRING));
    fav_open.connect_activate({
        let ui = ui.clone();
        move |_, param| {
            if let Some(p) = param.and_then(|v| v.get::<String>()) {
                load_archive(&ui, PathBuf::from(p));
            }
        }
    });
    group.add_action(&fav_open);
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
    let file_open = gio::Menu::new();
    file_open.append(Some("Buka Archive…"), Some("win.open"));
    file_open.append(Some("Simpan Salinan Archive…"), Some("win.save_copy"));
    file_open.append(Some("Set Password Default…"), Some("win.set_pw"));
    file_open.append(Some("Tutup Archive"), Some("win.close"));
    file.append_section(None, &file_open);
    let file_sel = gio::Menu::new();
    file_sel.append(Some("Pilih Semua"), Some("win.select_all"));
    file_sel.append(Some("Balik Seleksi"), Some("win.invert_sel"));
    file.append_section(None, &file_sel);
    let file_exit = gio::Menu::new();
    file_exit.append(Some("Keluar"), Some("win.quit"));
    file.append_section(None, &file_exit);
    menu.append_submenu(Some("File"), &file);

    let cmds = gio::Menu::new();
    cmds.append(Some("Tambah Berkas…"), Some("win.add"));
    cmds.append(Some("Extract Ke…"), Some("win.extract"));
    cmds.append(Some("Test"), Some("win.test"));
    cmds.append(Some("Ganti Nama…"), Some("win.rename"));
    cmds.append(Some("Hapus"), Some("win.delete"));
    cmds.append(Some("Komentar Archive…"), Some("win.comment"));
    menu.append_submenu(Some("Commands"), &cmds);

    let tools = gio::Menu::new();
    tools.append(Some("Wizard"), Some("win.wizard"));
    tools.append(Some("Pindai Virus…"), Some("win.scan"));
    tools.append(Some("Perbaiki Arsip…"), Some("win.repair"));
    tools.append(Some("Convert Archive…"), Some("win.convert"));
    tools.append(Some("Buat SFX (.sh)…"), Some("win.sfx"));
    tools.append(Some("Buat Laporan…"), Some("win.report"));
    tools.append(Some("Cari…"), Some("win.find"));
    menu.append_submenu(Some("Tools"), &tools);

    menu.append_submenu(Some("Favorites"), &ui.favorites_menu);
    refresh_favorites_menu(ui);

    let options = gio::Menu::new();
    options.append(Some("Preferensi…"), Some("win.options"));
    options.append(Some("Lihat Log…"), Some("win.log"));
    menu.append_submenu(Some("Options"), &options);

    let help = gio::Menu::new();
    help.append(Some("Tentang Zippy"), Some("win.about"));
    menu.append_submenu(Some("Help"), &help);

    gtk::PopoverMenuBar::from_model(Some(&menu))
}

/// Bangun ulang isi submenu Favorites dari daftar tersimpan.
fn refresh_favorites_menu(ui: &Rc<Ui>) {
    let m = &ui.favorites_menu;
    m.remove_all();

    let actions = gio::Menu::new();
    actions.append(Some("Tambah arsip saat ini"), Some("win.fav_add"));
    actions.append(Some("Hapus arsip saat ini"), Some("win.fav_remove"));
    actions.append(Some("Kelola Favorit…"), Some("win.fav_manage"));
    m.append_section(None, &actions);

    let favs = config::favorites_load();
    if favs.is_empty() {
        return;
    }
    let list = gio::Menu::new();
    for p in &favs {
        let label = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string());
        let item = gio::MenuItem::new(Some(&label), None);
        let target = p.to_string_lossy().into_owned().to_variant();
        item.set_action_and_target_value(Some("win.fav_open"), Some(&target));
        list.append_item(&item);
    }
    m.append_section(Some("Tersimpan"), &list);
}

fn fav_add_current(ui: &Rc<Ui>) {
    let Some(path) = ui.current.borrow().clone() else {
        show_toast(ui, "Buka arsip dulu sebelum menambah ke Favorit");
        return;
    };
    config::favorites_add(&path);
    refresh_favorites_menu(ui);
    show_toast(ui, "Ditambahkan ke Favorit");
}

fn fav_remove_current(ui: &Rc<Ui>) {
    let Some(path) = ui.current.borrow().clone() else {
        return;
    };
    if !config::favorites_contains(&path) {
        show_toast(ui, "Arsip ini tidak ada di Favorit");
        return;
    }
    config::favorites_remove(&path);
    refresh_favorites_menu(ui);
    show_toast(ui, "Dihapus dari Favorit");
}

/// Dialog kelola favorit: daftar dengan tombol buka & hapus per baris.
fn show_favorites_manager(ui: &Rc<Ui>) {
    let win = adw::PreferencesWindow::builder()
        .transient_for(&ui.window)
        .modal(true)
        .title("Kelola Favorit")
        .search_enabled(false)
        .build();
    win.set_default_size(460, 420);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("Arsip Favorit")
        .description("Klik baris untuk membuka, atau tombol hapus untuk membuang.")
        .build();

    let favs = config::favorites_load();
    if favs.is_empty() {
        let empty = adw::ActionRow::builder()
            .title("Belum ada favorit")
            .subtitle("Tambahkan lewat menu Favorites → Tambah arsip saat ini")
            .build();
        group.add(&empty);
    } else {
        for p in favs {
            let title = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string());
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(p.display().to_string())
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("application-x-archive"));

            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text("Hapus dari Favorit")
                .valign(gtk::Align::Center)
                .build();
            remove.add_css_class("flat");
            remove.connect_clicked({
                let ui = ui.clone();
                let p = p.clone();
                let win = win.clone();
                move |_| {
                    config::favorites_remove(&p);
                    refresh_favorites_menu(&ui);
                    win.close();
                    show_favorites_manager(&ui); // tampilkan ulang dengan daftar baru
                }
            });
            row.add_suffix(&remove);
            row.connect_activated({
                let ui = ui.clone();
                let p = p.clone();
                let win = win.clone();
                move |_| {
                    win.close();
                    load_archive(&ui, p.clone());
                }
            });
            group.add(&row);
        }
    }
    page.add(&group);
    win.add(&page);
    win.present();
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

    let add = tool_button("zippy-add", "Add");
    add.connect_clicked({
        let ui = ui.clone();
        move |_| compress_dialog(&ui)
    });
    extract_btn.connect_clicked({
        let ui = ui.clone();
        move |_| extract_dialog(&ui)
    });
    extract_btn.set_sensitive(false);

    let test = tool_button("zippy-test", "Test");
    test.connect_clicked({
        let ui = ui.clone();
        move |_| test_dialog(&ui)
    });
    let view = tool_button("zippy-view", "View");
    view.connect_clicked({
        let ui = ui.clone();
        move |_| view_selected(&ui)
    });
    let delete = tool_button("zippy-delete", "Delete");
    delete.connect_clicked({
        let ui = ui.clone();
        move |_| delete_selected(&ui)
    });
    let find = tool_button("zippy-find", "Find");
    find.connect_clicked({
        let ui = ui.clone();
        move |_| toggle_search(&ui)
    });
    let repair = tool_button("zippy-repair", "Repair");
    repair.connect_clicked({
        let ui = ui.clone();
        move |_| repair_dialog(&ui)
    });
    let scan = tool_button("zippy-scan", "Scan");
    scan.connect_clicked({
        let ui = ui.clone();
        move |_| scan_dialog(&ui)
    });
    let wizard = tool_button("zippy-wizard", "Wizard");
    wizard.connect_clicked({
        let ui = ui.clone();
        move |_| show_wizard(&ui)
    });
    let info = tool_button("zippy-info", "Info");
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
    bar.append(&repair);
    bar.append(&scan);
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
        let res = zippy_core::archive::list(&worker_path, None);
        // Peringatan enkripsi lemah hanya relevan bila list sukses.
        let weak = res.is_ok()
            && zippy_core::archive::has_weak_encryption(&worker_path).unwrap_or(false);
        let _ = tx.send_blocking((res, weak));
    });

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok((Ok(entries), weak)) => {
                let total = entries.len();
                *ui.entries.borrow_mut() = entries;
                ui.cwd.borrow_mut().clear();
                *ui.current.borrow_mut() = Some(path.clone());
                ui.extract_btn.set_sensitive(true);
                render(&ui);
                tracing::info!(entries = total, archive = %path.display(), "archive dibuka");
                if weak {
                    show_toast(
                        &ui,
                        "⚠ Archive memakai enkripsi ZipCrypto lemah — pertimbangkan ulang dengan AES-256",
                    );
                }
                if let Some(dir) = std::env::var_os("ZIPPY_EXTRACT_TO") {
                    let pw = std::env::var("ZIPPY_PASSWORD").ok();
                    run_extract(&ui, path.clone(), PathBuf::from(dir), pw, OverwriteMode::Overwrite);
                }
                if std::env::var_os("ZIPPY_TEST").is_some() {
                    run_test(&ui, path.clone(), std::env::var("ZIPPY_PASSWORD").ok());
                }
            }
            Ok((Err(e), _)) => {
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
                start_extract(&ui, archive.clone(), dest);
            }
        }
    });
}

/// Mulai extract arsip terbuka ke `dest`. Bila ada berkas tujuan yang sudah
/// ada, tanyakan dulu kebijakan overwrite (Overwrite/Skip/Rename); jika tidak
/// ada konflik, langsung jalan.
fn start_extract(ui: &Rc<Ui>, archive: PathBuf, dest: PathBuf) {
    let conflicts = count_conflicts(&ui.entries.borrow(), &dest);
    if conflicts == 0 {
        run_extract(ui, archive, dest, None, OverwriteMode::Overwrite);
    } else {
        ask_overwrite_mode(ui, archive, dest, conflicts);
    }
}

/// Hitung berapa berkas (non-folder) yang sudah ada di `dest`.
fn count_conflicts(entries: &[Entry], dest: &Path) -> usize {
    entries
        .iter()
        .filter(|e| !e.is_dir && dest.join(&e.name).exists())
        .count()
}

/// Dialog pilihan kebijakan overwrite saat ada konflik berkas.
fn ask_overwrite_mode(ui: &Rc<Ui>, archive: PathBuf, dest: PathBuf, conflicts: usize) {
    let body = format!(
        "{conflicts} berkas sudah ada di folder tujuan.\nPilih cara menanganinya:"
    );
    let dialog =
        adw::MessageDialog::new(Some(&ui.window), Some("Berkas Sudah Ada"), Some(&body));
    set_dialog_icon(&dialog, "zippy-bad");
    dialog.add_response("cancel", "Batal");
    dialog.add_response("skip", "Lewati");
    dialog.add_response("rename", "Beri Nama Baru");
    dialog.add_response("overwrite", "Timpa Semua");
    dialog.set_response_appearance("overwrite", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("overwrite"));
    dialog.set_close_response("cancel");

    let ui = ui.clone();
    dialog.connect_response(None, move |_, resp| {
        let mode = match resp {
            "overwrite" => OverwriteMode::Overwrite,
            "skip" => OverwriteMode::Skip,
            "rename" => OverwriteMode::Rename,
            _ => return,
        };
        run_extract(&ui, archive.clone(), dest.clone(), None, mode);
    });
    dialog.present();
}

/// Extract `archive` → `dest`. `password` dipakai bila archive terenkripsi;
/// bila `None` dan core melaporkan [`Error::Password`], UI memunculkan dialog
/// password lalu memanggil ulang dengan password yang dimasukkan.
fn run_extract(
    ui: &Rc<Ui>,
    archive: PathBuf,
    dest: PathBuf,
    password: Option<String>,
    mode: OverwriteMode,
) {
    let password = password.or_else(|| ui.default_password.borrow().clone());
    let prohibited = ui.config.borrow().prohibited.clone();
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
            zippy_core::archive::extract_all_with(
                &worker_archive,
                &worker_dest,
                worker_pw.as_deref(),
                mode,
                &prohibited,
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
                show_toast_open_folder(&ui, "Extract selesai", dest.clone());
                log_event(&ui, &format!("Extract: {} → {}", archive.display(), dest.display()));
                tracing::info!("extract selesai");
                // Hapus arsip ke Trash setelah sukses (bila diaktifkan & arsip
                // yang sedang dibuka).
                if ui.config.borrow().delete_after_extract {
                    trash_archive_after_extract(&ui, &archive);
                }
            }
            Ok(Err(Error::Cancelled)) => {
                show_toast(&ui, "Extract dibatalkan");
                tracing::info!("extract dibatalkan");
            }
            Ok(Err(Error::Password)) => {
                tracing::warn!("extract perlu password");
                prompt_password(&ui, &archive, &dest, mode);
            }
            Ok(Err(e)) => {
                show_result_dialog(&ui, Notif::Bad, "Extract Gagal", &e.to_string());
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
    dialog.set_extra_child(Some(&icon_with("zippy-info", &entry)));
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
fn prompt_password(ui: &Rc<Ui>, archive: &Path, dest: &Path, mode: OverwriteMode) {
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
            Some(pw) => run_extract(ui, archive.clone(), dest.clone(), Some(pw), mode),
            None => show_toast(ui, "Password kosong"),
        },
    );
}

// ---------------------------------------------------------------------------
// Gelombang 2: password default · simpan salinan · laporan · seleksi
// ---------------------------------------------------------------------------

/// File → Set default password. Disimpan di memori sesi (tidak persisten),
/// dipakai sebagai fallback oleh extract/test/view.
fn set_default_password(ui: &Rc<Ui>) {
    ask_password(
        ui,
        "Password Default",
        "Dipakai otomatis untuk extract/test/view arsip terenkripsi (sesi ini saja).",
        "Simpan",
        |ui, pw| {
            let set = pw.is_some();
            *ui.default_password.borrow_mut() = pw;
            show_toast(
                ui,
                if set {
                    "Password default diset"
                } else {
                    "Password default dikosongkan"
                },
            );
        },
    );
}

/// File → Simpan salinan archive sebagai… (copy file arsip apa adanya).
fn save_archive_copy(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => {
            show_toast(ui, "Belum ada archive terbuka");
            return;
        }
    };
    let dialog = gtk::FileDialog::builder()
        .title("Simpan salinan archive sebagai…")
        .build();
    if let Some(name) = archive.file_name() {
        dialog.set_initial_name(Some(&name.to_string_lossy()));
    }
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.save(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(dest) = file.path() {
                if dest == archive {
                    show_toast(&ui, "Tujuan sama dengan sumber");
                    return;
                }
                match std::fs::copy(&archive, &dest) {
                    Ok(_) => show_toast(&ui, "Salinan archive disimpan"),
                    Err(e) => show_toast(&ui, &format!("Gagal menyimpan: {e}")),
                }
            }
        }
    });
}

/// Tools → Generate report: tulis daftar isi + ringkasan ke berkas teks.
fn generate_report(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => {
            show_toast(ui, "Belum ada archive terbuka");
            return;
        }
    };
    let entries = ui.entries.borrow().clone();
    if entries.is_empty() {
        show_toast(ui, "Archive kosong");
        return;
    }
    let report = build_report(&archive, &entries);
    let stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
    let dialog = gtk::FileDialog::builder().title("Simpan laporan…").build();
    dialog.set_initial_name(Some(&format!("{stem}-laporan.txt")));
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.save(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(dest) = file.path() {
                match std::fs::write(&dest, report.as_bytes()) {
                    Ok(_) => {
                        let dir = dest.parent().map(Path::to_path_buf).unwrap_or(dest);
                        show_toast_open_folder(&ui, "Laporan disimpan", dir);
                    }
                    Err(e) => show_toast(&ui, &format!("Gagal menulis laporan: {e}")),
                }
            }
        }
    });
}

/// Bangun isi laporan teks (header + ringkasan + tabel TSV).
fn build_report(archive: &Path, entries: &[Entry]) -> String {
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let packed: u64 = entries.iter().map(|e| e.compressed_size).sum();
    let files = entries.iter().filter(|e| !e.is_dir).count();
    let dirs = entries.iter().filter(|e| e.is_dir).count();

    let mut s = String::new();
    s.push_str(&format!("Laporan Archive — Zippy v{}\n", zippy_core::VERSION));
    s.push_str(&format!("Archive : {}\n", archive.display()));
    s.push_str(&format!("Berkas  : {files}   Folder: {dirs}\n"));
    s.push_str(&format!(
        "Ukuran  : {} bytes (packed {} bytes",
        file_list::group_thousands(total),
        file_list::group_thousands(packed)
    ));
    if total > 0 {
        s.push_str(&format!(", rasio {:.1}%", packed as f64 / total as f64 * 100.0));
    }
    s.push_str(")\n\n");
    s.push_str("Nama\tUkuran\tPacked\tModified\tCRC32\n");
    for e in entries {
        let crc = e.crc32.map(|c| format!("{c:08X}")).unwrap_or_default();
        s.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            e.name,
            e.size,
            e.compressed_size,
            e.modified.clone().unwrap_or_default(),
            crc
        ));
    }
    s
}

/// File → Pilih semua baris di folder yang sedang ditampilkan.
fn select_all_rows(ui: &Rc<Ui>) {
    if let Some(sel) = ui.list.column_view.model().and_downcast::<gtk::MultiSelection>() {
        sel.select_all();
    }
}

/// File → Balik seleksi.
fn invert_selection(ui: &Rc<Ui>) {
    let Some(sel) = ui.list.column_view.model().and_downcast::<gtk::MultiSelection>() else {
        return;
    };
    for i in 0..sel.n_items() {
        if sel.is_selected(i) {
            sel.unselect_item(i);
        } else {
            sel.select_item(i, false);
        }
    }
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

/// Urutan item dropdown level ↔ [`Level`] core. Indeks dipakai dua arah.
const LEVEL_LABELS: [&str; 4] = [
    "Simpan (tanpa kompresi)",
    "Cepat",
    "Normal",
    "Maksimal",
];

fn level_from_index(i: u32) -> Level {
    match i {
        0 => Level::Store,
        1 => Level::Fastest,
        3 => Level::Best,
        _ => Level::Normal,
    }
}

fn index_from_level(l: Level) -> u32 {
    match l {
        Level::Store => 0,
        Level::Fastest => 1,
        Level::Normal => 2,
        Level::Best => 3,
    }
}

/// Dialog opsi compress: pilih tingkat kompresi dan (untuk zip/7z) password
/// enkripsi AES-256. Untuk `.tar` polos (tanpa kompresi & tanpa enkripsi)
/// langsung jalan tanpa dialog.
fn compress_to(ui: &Rc<Ui>, inputs: Vec<PathBuf>, dest: PathBuf) {
    let kind = zippy_core::archive::kind_from_ext(&dest);
    let supports_pw = matches!(kind, Some(ArchiveKind::Zip | ArchiveKind::SevenZip));
    let supports_level = !matches!(kind, Some(ArchiveKind::Tar));

    // Plain tar: tak ada yang bisa diatur → langsung kompres.
    if !supports_pw && !supports_level {
        run_compress(ui, inputs, dest, Level::default(), None, false);
        return;
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);

    let level_dropdown = gtk::DropDown::from_strings(&LEVEL_LABELS);
    level_dropdown.set_selected(index_from_level(ui.config.borrow().level));
    if supports_level {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&gtk::Label::new(Some("Tingkat kompresi:")));
        level_dropdown.set_hexpand(true);
        row.append(&level_dropdown);
        content.append(&row);
    }

    let pw_entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .activates_default(true)
        .build();
    if supports_pw {
        pw_entry.set_placeholder_text(Some("Password AES-256 (opsional)"));
        content.append(&pw_entry);
    }

    let del_check = gtk::CheckButton::with_label("Hapus berkas sumber setelah arsip dibuat");
    content.append(&del_check);

    let dialog = adw::MessageDialog::new(Some(&ui.window), Some("Buat Archive"), None);
    dialog.set_extra_child(Some(&icon_with("zippy-add", &content)));
    dialog.add_response("cancel", "Batal");
    dialog.add_response("ok", "Buat");
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let ui = ui.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "ok" {
            return;
        }
        let level = if supports_level {
            level_from_index(level_dropdown.selected())
        } else {
            Level::default()
        };
        let pw = if supports_pw {
            let t = pw_entry.text().to_string();
            if t.is_empty() { None } else { Some(t) }
        } else {
            None
        };
        run_compress(&ui, inputs.clone(), dest.clone(), level, pw, del_check.is_active());
    });
    dialog.present();
}

fn run_compress(
    ui: &Rc<Ui>,
    inputs: Vec<PathBuf>,
    dest: PathBuf,
    level: Level,
    password: Option<String>,
    delete_after: bool,
) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());
    let sources = inputs.clone();
    let dest_label = dest.clone();

    let (tx_ev, rx_ev) = async_channel::unbounded();
    let (tx_done, rx_done) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let res = {
            let sink = ChannelSink::new(tx_ev);
            let refs: Vec<&Path> = inputs.iter().map(|p| p.as_path()).collect();
            zippy_core::archive::compress_with_level(
                &refs,
                &dest,
                password.as_deref(),
                level,
                &cancel,
                &sink,
            )
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
            Ok(Ok(())) => {
                log_event(&ui, &format!("Arsip dibuat: {}", dest_label.display()));
                if delete_after {
                    trash_sources(&ui, &sources);
                }
                show_toast(&ui, "Archive dibuat");
            }
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Kompres dibatalkan"),
            Ok(Err(e)) => show_result_dialog(&ui, Notif::Bad, "Kompres Gagal", &e.to_string()),
            Err(_) => {}
        }
    });
}

/// Pindahkan daftar berkas/folder sumber ke Trash (dipakai "Hapus sumber
/// setelah arsip"). Lapor jumlah yang gagal bila ada.
fn trash_sources(ui: &Rc<Ui>, sources: &[PathBuf]) {
    let mut failed = 0;
    for p in sources {
        if gio::File::for_path(p).trash(gio::Cancellable::NONE).is_err() {
            failed += 1;
        }
    }
    if failed == 0 {
        log_event(ui, &format!("{} sumber dipindah ke Trash", sources.len()));
        show_toast(ui, "Berkas sumber dipindahkan ke Trash");
    } else {
        show_toast(ui, &format!("{failed} sumber gagal dipindah ke Trash"));
    }
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
    let password = password.or_else(|| ui.default_password.borrow().clone());
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
                show_result_dialog(
                    &ui,
                    Notif::Good,
                    "Test Selesai",
                    "Tidak ada kesalahan ditemukan — arsip utuh.",
                );
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
            Ok(Err(e)) => show_result_dialog(
                &ui,
                Notif::Bad,
                "Test Gagal",
                &format!("Arsip rusak atau tidak valid:\n{e}"),
            ),
            Err(_) => {}
        }
    });
}

// ---------------------------------------------------------------------------
// Repair archive (zip -FF / sidecar PAR2)
// ---------------------------------------------------------------------------

fn repair_dialog(ui: &Rc<Ui>) {
    match ui.current.borrow().clone() {
        Some(archive) => run_repair(ui, archive),
        None => show_toast(ui, "Belum ada archive terbuka"),
    }
}

fn run_repair(ui: &Rc<Ui>, archive: PathBuf) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_done, rx_done) = async_channel::bounded(1);
    let worker = archive.clone();
    std::thread::spawn(move || {
        let _ = tx_done.send_blocking(zippy_core::repair(&worker, &cancel));
    });

    let pulse = start_pulse(ui, "Memperbaiki arsip…");
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let res = rx_done.recv().await;
        stop_pulse(&ui, &pulse);
        match res {
            Ok(Ok(rep)) => show_repair_result(&ui, &rep),
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Perbaikan dibatalkan"),
            Ok(Err(e)) => show_result_dialog(&ui, Notif::Bad, "Repair Gagal", &e.to_string()),
            Err(_) => {}
        }
    });
}

fn show_repair_result(ui: &Rc<Ui>, rep: &zippy_core::RepairReport) {
    let mut body = format!("Metode: {}\n", rep.method);
    if let Some(p) = &rep.output_path {
        body.push_str(&format!("Output: {}\n", p.display()));
    }
    body.push_str(if rep.repaired {
        "\nStatus: berhasil ✓"
    } else {
        "\nStatus: tidak dapat diperbaiki sepenuhnya"
    });
    let (heading, kind) = if rep.repaired {
        ("Perbaikan Berhasil", Notif::Good)
    } else {
        ("Perbaikan Tidak Tuntas", Notif::Bad)
    };
    show_result_dialog(ui, kind, heading, &body);
}

// ---------------------------------------------------------------------------
// Scan virus (ClamAV)
// ---------------------------------------------------------------------------

fn scan_dialog(ui: &Rc<Ui>) {
    if zippy_core::virus_scanner().is_none() {
        show_result_dialog(
            ui,
            Notif::Bad,
            "ClamAV Tidak Terpasang",
            "Pemindaian virus butuh ClamAV. Pasang paket `clamav` lalu coba lagi.",
        );
        return;
    }
    match ui.current.borrow().clone() {
        Some(archive) => run_scan(ui, archive),
        None => show_toast(ui, "Belum ada archive terbuka"),
    }
}

fn run_scan(ui: &Rc<Ui>, archive: PathBuf) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_done, rx_done) = async_channel::bounded(1);
    let worker = archive.clone();
    std::thread::spawn(move || {
        let _ = tx_done.send_blocking(zippy_core::scan(&worker, &cancel));
    });

    let pulse = start_pulse(ui, "Memindai virus…");
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let res = rx_done.recv().await;
        stop_pulse(&ui, &pulse);
        match res {
            Ok(Ok(rep)) => show_scan_result(&ui, &rep),
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Pemindaian dibatalkan"),
            Ok(Err(e)) => show_result_dialog(&ui, Notif::Bad, "Scan Gagal", &e.to_string()),
            Err(_) => {}
        }
    });
}

fn show_scan_result(ui: &Rc<Ui>, rep: &zippy_core::ScanReport) {
    let clean = rep.is_clean();
    let mut body = format!("Scanner: {}\n", rep.scanner);
    if clean {
        body.push_str("\nArsip bersih ✓");
    } else {
        body.push_str(&format!("\n{} berkas terinfeksi:\n", rep.findings.len()));
        for f in rep.findings.iter().take(20) {
            body.push_str(&format!("• {} — {}\n", f.path, f.signature));
        }
        if rep.findings.len() > 20 {
            body.push_str(&format!("… dan {} lagi\n", rep.findings.len() - 20));
        }
    }
    let (heading, kind) = if clean {
        ("Tidak Ada Virus", Notif::Good)
    } else {
        ("Virus Terdeteksi!", Notif::Bad)
    };
    show_result_dialog(ui, kind, heading, &body);
}

// ---------------------------------------------------------------------------
// Convert (ubah format) + Convert to SFX
// ---------------------------------------------------------------------------

fn convert_dialog(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => {
            show_toast(ui, "Belum ada archive terbuka");
            return;
        }
    };
    let stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
    let dialog = gtk::FileDialog::builder()
        .title("Convert ke… (format dari ekstensi)")
        .initial_name(format!("{stem}.7z"))
        .build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.save(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(dest) = file.path() {
                ask_convert_options(&ui, archive.clone(), dest);
            }
        }
    });
}

/// Dialog tingkat kompresi + password (untuk zip/7z) sebelum konversi.
fn ask_convert_options(ui: &Rc<Ui>, src: PathBuf, dest: PathBuf) {
    let kind = zippy_core::archive::kind_from_ext(&dest);
    if kind.is_none() {
        show_result_dialog(ui, Notif::Bad, "Format Tidak Dikenali", "Ekstensi tujuan tidak didukung.");
        return;
    }
    let supports_pw = matches!(kind, Some(ArchiveKind::Zip | ArchiveKind::SevenZip));
    let supports_level = !matches!(kind, Some(ArchiveKind::Tar));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let level_dropdown = gtk::DropDown::from_strings(&LEVEL_LABELS);
    level_dropdown.set_selected(index_from_level(ui.config.borrow().level));
    if supports_level {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&gtk::Label::new(Some("Tingkat kompresi:")));
        level_dropdown.set_hexpand(true);
        row.append(&level_dropdown);
        content.append(&row);
    }
    let pw_entry = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    if supports_pw {
        pw_entry.set_placeholder_text(Some("Password AES-256 hasil (opsional)"));
        content.append(&pw_entry);
    }

    let dialog = adw::MessageDialog::new(Some(&ui.window), Some("Convert Archive"), None);
    dialog.set_extra_child(Some(&icon_with("zippy-add", &content)));
    dialog.add_response("cancel", "Batal");
    dialog.add_response("ok", "Convert");
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let ui = ui.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "ok" {
            return;
        }
        let level = if supports_level {
            level_from_index(level_dropdown.selected())
        } else {
            Level::default()
        };
        let dest_pw = if supports_pw {
            let t = pw_entry.text().to_string();
            if t.is_empty() { None } else { Some(t) }
        } else {
            None
        };
        run_convert(&ui, src.clone(), dest.clone(), None, dest_pw, level);
    });
    dialog.present();
}

fn run_convert(
    ui: &Rc<Ui>,
    src: PathBuf,
    dest: PathBuf,
    src_pw: Option<String>,
    dest_pw: Option<String>,
    level: Level,
) {
    let src_pw = src_pw.or_else(|| ui.default_password.borrow().clone());
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_done, rx_done) = async_channel::bounded(1);
    let (ws, wd, wsp, wdp) = (src.clone(), dest.clone(), src_pw.clone(), dest_pw.clone());
    std::thread::spawn(move || {
        let res = zippy_core::archive::convert(
            &ws,
            &wd,
            wsp.as_deref(),
            wdp.as_deref(),
            level,
            &cancel,
            &zippy_core::NullSink,
        );
        let _ = tx_done.send_blocking(res);
    });

    let pulse = start_pulse(ui, "Mengonversi…");
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let res = rx_done.recv().await;
        stop_pulse(&ui, &pulse);
        match res {
            Ok(Ok(())) => {
                log_event(&ui, &format!("Convert: {} → {}", src.display(), dest.display()));
                show_result_dialog(&ui, Notif::Good, "Konversi Selesai", &format!("Arsip dibuat:\n{}", dest.display()));
            }
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Konversi dibatalkan"),
            Ok(Err(Error::Password)) => ask_password(
                &ui,
                "Sumber Terenkripsi",
                "Masukkan password untuk membuka arsip sumber.",
                "Convert",
                move |ui, pw| match pw {
                    Some(pw) => run_convert(ui, src.clone(), dest.clone(), Some(pw), dest_pw.clone(), level),
                    None => show_toast(ui, "Password kosong"),
                },
            ),
            Ok(Err(e)) => show_result_dialog(&ui, Notif::Bad, "Konversi Gagal", &e.to_string()),
            Err(_) => {}
        }
    });
}

fn sfx_dialog(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => {
            show_toast(ui, "Belum ada archive terbuka");
            return;
        }
    };
    let stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
    let dialog = gtk::FileDialog::builder()
        .title("Buat SFX (.sh) ke…")
        .initial_name(format!("{stem}.sh"))
        .build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.save(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(dest) = file.path() {
                run_sfx(&ui, archive.clone(), dest, None);
            }
        }
    });
}

fn run_sfx(ui: &Rc<Ui>, src: PathBuf, dest: PathBuf, src_pw: Option<String>) {
    let src_pw = src_pw.or_else(|| ui.default_password.borrow().clone());
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_done, rx_done) = async_channel::bounded(1);
    let (ws, wd, wsp) = (src.clone(), dest.clone(), src_pw.clone());
    std::thread::spawn(move || {
        let res = zippy_core::make_sfx(&ws, &wd, wsp.as_deref(), &cancel, &zippy_core::NullSink);
        let _ = tx_done.send_blocking(res);
    });

    let pulse = start_pulse(ui, "Membuat SFX…");
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let res = rx_done.recv().await;
        stop_pulse(&ui, &pulse);
        match res {
            Ok(Ok(())) => {
                log_event(&ui, &format!("SFX dibuat: {}", dest.display()));
                show_result_dialog(
                    &ui,
                    Notif::Good,
                    "SFX Dibuat",
                    &format!("Self-extracting script:\n{}\n\nJalankan: sh {} [folder-tujuan]", dest.display(), dest.display()),
                );
            }
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Pembuatan SFX dibatalkan"),
            Ok(Err(Error::Password)) => ask_password(
                &ui,
                "Sumber Terenkripsi",
                "Masukkan password untuk membuka arsip sumber.",
                "Buat SFX",
                move |ui, pw| match pw {
                    Some(pw) => run_sfx(ui, src.clone(), dest.clone(), Some(pw)),
                    None => show_toast(ui, "Password kosong"),
                },
            ),
            Ok(Err(e)) => show_result_dialog(&ui, Notif::Bad, "SFX Gagal", &e.to_string()),
            Err(_) => {}
        }
    });
}

// ---------------------------------------------------------------------------
// Komentar arsip (ZIP)
// ---------------------------------------------------------------------------

fn comment_dialog(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => {
            show_toast(ui, "Belum ada archive terbuka");
            return;
        }
    };
    if !matches!(
        zippy_core::archive::detect_kind(&archive),
        Ok(ArchiveKind::Zip)
    ) {
        show_result_dialog(ui, Notif::Bad, "Tidak Didukung", "Komentar arsip hanya tersedia untuk ZIP.");
        return;
    }
    let current = zippy_core::archive::read_comment(&archive).unwrap_or_default();

    let view = gtk::TextView::new();
    view.buffer().set_text(&current);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(160)
        .min_content_width(380)
        .child(&view)
        .build();

    let dialog = adw::MessageDialog::new(
        Some(&ui.window),
        Some("Komentar Archive"),
        Some("Komentar disimpan di arsip ZIP."),
    );
    dialog.set_extra_child(Some(&icon_with("zippy-info", &scroll)));
    dialog.add_response("cancel", "Batal");
    dialog.add_response("ok", "Simpan");
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let ui = ui.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "ok" {
            return;
        }
        let buf = view.buffer();
        let text = buf
            .text(&buf.start_iter(), &buf.end_iter(), false)
            .to_string();
        run_set_comment(&ui, archive.clone(), text);
    });
    dialog.present();
}

fn run_set_comment(ui: &Rc<Ui>, archive: PathBuf, comment: String) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());
    let (tx_done, rx_done) = async_channel::bounded(1);
    let wa = archive.clone();
    std::thread::spawn(move || {
        let res =
            zippy_core::archive::set_comment(&wa, &comment, &cancel, &zippy_core::NullSink);
        let _ = tx_done.send_blocking(res);
    });
    let pulse = start_pulse(ui, "Menyimpan komentar…");
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let res = rx_done.recv().await;
        stop_pulse(&ui, &pulse);
        match res {
            Ok(Ok(())) => show_toast(&ui, "Komentar disimpan"),
            Ok(Err(e)) => show_result_dialog(&ui, Notif::Bad, "Gagal Simpan Komentar", &e.to_string()),
            Err(_) => {}
        }
    });
}

/// Jenis ikon untuk dialog/notifikasi — menentukan ikon yang ditempel.
#[derive(Clone, Copy)]
enum Notif {
    /// Operasi sukses / hasil positif.
    Good,
    /// Gagal / peringatan / hasil negatif.
    Bad,
}

impl Notif {
    fn icon(self) -> &'static str {
        match self {
            Notif::Good => "zippy-good",
            Notif::Bad => "zippy-bad",
        }
    }
}

/// Tempelkan ikon notifikasi ke dialog (sebagai extra-child, 64px).
fn set_notif_icon(dialog: &adw::MessageDialog, kind: Notif) {
    set_dialog_icon(dialog, kind.icon());
}

/// Tempelkan ikon bernama `icon` ke dialog (extra-child, 64px). Untuk dialog
/// tanpa konten tambahan lain.
fn set_dialog_icon(dialog: &adw::MessageDialog, icon: &str) {
    let img = gtk::Image::from_icon_name(icon);
    img.set_pixel_size(64);
    dialog.set_extra_child(Some(&img));
}

/// Bungkus `content` dengan ikon di kiri — untuk dipasang sebagai extra-child
/// dialog yang juga memiliki widget input (mis. entry password / opsi compress).
fn icon_with(icon: &str, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let img = gtk::Image::from_icon_name(icon);
    img.set_pixel_size(48);
    img.set_valign(gtk::Align::Start);
    row.append(&img);
    let content = content.as_ref();
    content.set_hexpand(true);
    row.append(content);
    row
}

/// Dialog hasil/peringatan ber-ikon dengan satu tombol "Tutup" — pengganti
/// toast untuk pesan penting (gaya konsisten dgn dialog Repair/Scan).
fn show_result_dialog(ui: &Rc<Ui>, kind: Notif, heading: &str, body: &str) {
    let dialog = adw::MessageDialog::new(Some(&ui.window), Some(heading), Some(body));
    set_notif_icon(&dialog, kind);
    dialog.add_response("ok", "Tutup");
    dialog.set_default_response(Some("ok"));
    log_event(ui, &format!("{heading} — {body}"));
    dialog.present();
}

/// Tampilkan progress indeterminate (pulse) untuk operasi tanpa event per-file.
/// Mengembalikan flag; panggil [`stop_pulse`] saat selesai.
fn start_pulse(ui: &Rc<Ui>, label: &str) -> Rc<std::cell::Cell<bool>> {
    ui.revealer.set_reveal_child(true);
    ui.cancel_btn.set_sensitive(true);
    ui.bar.set_fraction(0.0);
    ui.progress_label.set_text(label);
    schedule_dev_cancel(ui);

    let flag = Rc::new(std::cell::Cell::new(true));
    let ui_bar = ui.clone();
    let f = flag.clone();
    glib::timeout_add_local(Duration::from_millis(120), move || {
        if f.get() {
            ui_bar.bar.pulse();
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
    flag
}

fn stop_pulse(ui: &Rc<Ui>, flag: &Rc<std::cell::Cell<bool>>) {
    flag.set(false);
    ui.revealer.set_reveal_child(false);
    *ui.cancel.borrow_mut() = None;
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
    let password = password.or_else(|| ui.default_password.borrow().clone());
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
            s2.append(Some("Ganti nama…"), Some("win.rename"));
            s2.append(Some("Salin nama"), Some("win.copy_name"));
            s2.append(Some("Hapus folder ini"), Some("win.delete"));
            s2.append(Some("Properti…"), Some("win.props"));
            menu.append_section(None, &s2);
        }
        [_o] => {
            let s = gio::Menu::new();
            s.append(Some("Buka (View)"), Some("win.view"));
            s.append(Some("Extract berkas ini…"), Some("win.extract_sel"));
            menu.append_section(None, &s);
            let s2 = gio::Menu::new();
            s2.append(Some("Ganti nama…"), Some("win.rename"));
            s2.append(Some("Salin nama"), Some("win.copy_name"));
            s2.append(Some("Hapus"), Some("win.delete"));
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
            s.append(
                Some(&format!("Hapus {} item terpilih", many.len())),
                Some("win.delete"),
            );
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
    set_dialog_icon(&dialog, "zippy-info");
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
// Delete (hapus entri in-place)
// ---------------------------------------------------------------------------

fn delete_selected(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => return,
    };

    // Format yang tidak mendukung hapus: tolak lebih awal dengan pesan jelas.
    match zippy_core::archive::kind_from_ext(&archive) {
        Some(ArchiveKind::Rar) => {
            show_toast(ui, "RAR tidak mendukung hapus (extract-only)");
            return;
        }
        Some(ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst) => {
            show_toast(ui, "Format stream tunggal tak punya entri untuk dihapus");
            return;
        }
        _ => {}
    }

    // Kumpulkan path entri terpilih (folder cukup pathnya — core menghapus
    // isinya secara rekursif). Baris ".." diabaikan.
    let names: Vec<String> = selected_entries(ui)
        .iter()
        .filter(|o| !o.is_parent())
        .map(|o| o.full_path())
        .collect();
    if names.is_empty() {
        show_toast(ui, "Pilih entri yang akan dihapus");
        return;
    }

    // Hormati preferensi: lewati konfirmasi bila dimatikan di Options.
    if !ui.config.borrow().confirm_delete {
        run_delete(ui, archive, names, None);
        return;
    }

    let body = if names.len() == 1 {
        format!("Hapus \"{}\" dari archive? Tindakan ini tidak bisa dibatalkan.", names[0])
    } else {
        format!("Hapus {} item dari archive? Tindakan ini tidak bisa dibatalkan.", names.len())
    };
    let dialog = adw::MessageDialog::new(Some(&ui.window), Some("Hapus dari Archive"), Some(&body));
    set_dialog_icon(&dialog, "zippy-delete");
    dialog.add_response("cancel", "Batal");
    dialog.add_response("delete", "Hapus");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let ui = ui.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp == "delete" {
            run_delete(&ui, archive.clone(), names.clone(), None);
        }
    });
    dialog.present();
}

fn run_delete(ui: &Rc<Ui>, archive: PathBuf, names: Vec<String>, password: Option<String>) {
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_ev, rx_ev) = async_channel::unbounded();
    let (tx_done, rx_done) = async_channel::bounded(1);
    let worker_archive = archive.clone();
    let worker_names = names.clone();
    let worker_pw = password.clone();
    std::thread::spawn(move || {
        let res = {
            let sink = ChannelSink::new(tx_ev);
            let refs: Vec<&str> = worker_names.iter().map(|s| s.as_str()).collect();
            zippy_core::archive::delete(
                &worker_archive,
                &refs,
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
    ui.progress_label.set_text("Menghapus…");

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
                show_toast(&ui, "Entri dihapus");
                // Muat ulang agar daftar mencerminkan archive baru.
                load_archive(&ui, archive.clone());
            }
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Hapus dibatalkan"),
            Ok(Err(Error::Password)) => {
                let archive = archive.clone();
                let names = names.clone();
                ask_password(
                    &ui,
                    "Archive Terenkripsi",
                    "Masukkan password untuk menghapus entri.",
                    "Hapus",
                    move |ui, pw| match pw {
                        Some(pw) => run_delete(ui, archive.clone(), names.clone(), Some(pw)),
                        None => show_toast(ui, "Password kosong"),
                    },
                );
            }
            Ok(Err(e)) => show_toast(&ui, &format!("Gagal hapus: {e}")),
            Err(_) => {}
        }
    });
}

// ---------------------------------------------------------------------------
// Rename in-archive
// ---------------------------------------------------------------------------

fn rename_selected(ui: &Rc<Ui>) {
    let archive = match ui.current.borrow().clone() {
        Some(p) => p,
        None => {
            show_toast(ui, "Belum ada archive terbuka");
            return;
        }
    };
    match zippy_core::archive::kind_from_ext(&archive) {
        Some(ArchiveKind::Rar) => {
            show_toast(ui, "RAR tidak mendukung rename (extract-only)");
            return;
        }
        Some(ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst) => {
            show_toast(ui, "Format stream tunggal tak punya entri untuk di-rename");
            return;
        }
        _ => {}
    }
    let obj = match selected_entry(ui) {
        Some(o) => o,
        None => {
            show_toast(ui, "Pilih entri yang akan di-rename");
            return;
        }
    };
    if obj.is_parent() {
        return;
    }
    let old_full = obj.full_path();

    let entry = gtk::Entry::new();
    entry.set_text(&obj.name());
    entry.set_activates_default(true);
    let dialog = adw::MessageDialog::new(
        Some(&ui.window),
        Some("Ganti Nama"),
        Some("Masukkan nama baru (tetap di folder yang sama)."),
    );
    dialog.set_extra_child(Some(&icon_with("zippy-info", &entry)));
    dialog.add_response("cancel", "Batal");
    dialog.add_response("ok", "Ganti Nama");
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let ui = ui.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "ok" {
            return;
        }
        let new = entry.text().to_string();
        if new.trim().is_empty() {
            show_toast(&ui, "Nama baru kosong");
            return;
        }
        run_rename(&ui, archive.clone(), old_full.clone(), new, None);
    });
    dialog.present();
}

fn run_rename(ui: &Rc<Ui>, archive: PathBuf, old: String, new: String, password: Option<String>) {
    let password = password.or_else(|| ui.default_password.borrow().clone());
    let cancel = CancelToken::new();
    *ui.cancel.borrow_mut() = Some(cancel.clone());

    let (tx_done, rx_done) = async_channel::bounded(1);
    let (wa, wo, wn, wp) = (archive.clone(), old.clone(), new.clone(), password.clone());
    std::thread::spawn(move || {
        let res = zippy_core::archive::rename(
            &wa,
            &wo,
            &wn,
            wp.as_deref(),
            &cancel,
            &zippy_core::NullSink,
        );
        let _ = tx_done.send_blocking(res);
    });

    let pulse = start_pulse(ui, "Mengganti nama…");
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let res = rx_done.recv().await;
        stop_pulse(&ui, &pulse);
        match res {
            Ok(Ok(())) => {
                log_event(&ui, &format!("Rename: {old} → {new}"));
                show_toast(&ui, "Nama diubah");
                load_archive(&ui, archive.clone());
            }
            Ok(Err(Error::Cancelled)) => show_toast(&ui, "Rename dibatalkan"),
            Ok(Err(Error::Password)) => ask_password(
                &ui,
                "Archive Terenkripsi",
                "Masukkan password untuk mengganti nama entri.",
                "Ganti Nama",
                move |ui, pw| match pw {
                    Some(pw) => run_rename(ui, archive.clone(), old.clone(), new.clone(), Some(pw)),
                    None => show_toast(ui, "Password kosong"),
                },
            ),
            Ok(Err(e)) => show_result_dialog(&ui, Notif::Bad, "Rename Gagal", &e.to_string()),
            Err(_) => {}
        }
    });
}

// ---------------------------------------------------------------------------
// util
// ---------------------------------------------------------------------------

fn show_about(ui: &Rc<Ui>) {
    let about = adw::AboutWindow::builder()
        .transient_for(&ui.window)
        .modal(true)
        .application_name("Zippy")
        .application_icon(APP_ICON)
        .version(zippy_core::VERSION)
        .developer_name("s4rt4")
        .developers(vec!["s4rt4 <https://github.com/s4rt4>".to_string()])
        .comments(
            "Archive manager ringan untuk Linux — seringan WinRAR, dengan context menu \
             klik-kanan yang kaya & kondisional.\n\n\
             GTK4 + libadwaita di atas core Rust murni (zip/tar native, 7z/rar via subprocess).",
        )
        .website("https://github.com/s4rt4/zippy")
        .issue_url("https://github.com/s4rt4/zippy/issues")
        .license_type(gtk::License::MitX11)
        .copyright("© 2026 s4rt4")
        .build();
    about.add_link("Repositori GitHub", "https://github.com/s4rt4/zippy");
    about.add_link("Laporkan Masalah", "https://github.com/s4rt4/zippy/issues");
    about.present();
}

/// Daftarkan ikon aplikasi yang di-embed ke icon theme + tulis salinan ke
/// cache, agar logo & ikon judul muncul walau app belum di-install.
fn setup_icon_theme() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let base = cache_dir().join("zippy-icons");
    let dir = base.join("hicolor/scalable/apps");
    if std::fs::create_dir_all(&dir).is_ok() {
        let f = dir.join("io.github.s4rt4.Zippy.svg");
        let _ = std::fs::write(&f, APP_ICON_SVG);
    }
    // Ikon aksi toolbar berwarna (kategori "actions").
    let adir = base.join("hicolor/scalable/actions");
    if std::fs::create_dir_all(&adir).is_ok() {
        for (name, svg) in ACTION_ICONS {
            let _ = std::fs::write(adir.join(format!("{name}.svg")), svg);
        }
    }
    gtk::IconTheme::for_display(&display).add_search_path(&base);
}

fn cache_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x);
        }
    }
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".cache"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Terapkan skema warna libadwaita.
fn apply_scheme(s: config::Scheme) {
    let scheme = match s {
        config::Scheme::Default => adw::ColorScheme::Default,
        config::Scheme::Light => adw::ColorScheme::ForceLight,
        config::Scheme::Dark => adw::ColorScheme::ForceDark,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

// ---------------------------------------------------------------------------
// Preferensi (Options)
// ---------------------------------------------------------------------------

fn show_preferences(ui: &Rc<Ui>) {
    let win = adw::PreferencesWindow::builder()
        .transient_for(&ui.window)
        .modal(true)
        .title("Preferensi")
        .search_enabled(false)
        .build();
    win.set_default_size(480, 360);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder().title("Umum").build();

    // Tema.
    let scheme_row = adw::ComboRow::builder().title("Tema").build();
    let scheme_model = gtk::StringList::new(&["Ikuti sistem", "Terang", "Gelap"]);
    scheme_row.set_model(Some(&scheme_model));
    scheme_row.set_selected(match ui.config.borrow().scheme {
        config::Scheme::Default => 0,
        config::Scheme::Light => 1,
        config::Scheme::Dark => 2,
    });
    scheme_row.connect_selected_notify({
        let ui = ui.clone();
        move |r| {
            let s = match r.selected() {
                1 => config::Scheme::Light,
                2 => config::Scheme::Dark,
                _ => config::Scheme::Default,
            };
            apply_scheme(s);
            let mut c = ui.config.borrow_mut();
            c.scheme = s;
            c.save();
        }
    });
    group.add(&scheme_row);

    // Tingkat kompresi default.
    let level_row = adw::ComboRow::builder()
        .title("Tingkat kompresi default")
        .subtitle("Dipakai sebagai pilihan awal di dialog Add")
        .build();
    let level_model = gtk::StringList::new(&LEVEL_LABELS);
    level_row.set_model(Some(&level_model));
    level_row.set_selected(index_from_level(ui.config.borrow().level));
    level_row.connect_selected_notify({
        let ui = ui.clone();
        move |r| {
            let mut c = ui.config.borrow_mut();
            c.level = level_from_index(r.selected());
            c.save();
        }
    });
    group.add(&level_row);

    // Konfirmasi hapus.
    let del_row = adw::SwitchRow::builder()
        .title("Konfirmasi sebelum hapus")
        .subtitle("Tampilkan dialog konfirmasi saat menghapus entri arsip")
        .build();
    del_row.set_active(ui.config.borrow().confirm_delete);
    del_row.connect_active_notify({
        let ui = ui.clone();
        move |r| {
            let mut c = ui.config.borrow_mut();
            c.confirm_delete = r.is_active();
            c.save();
        }
    });
    group.add(&del_row);

    // Hapus arsip ke Trash setelah extract sukses.
    let trash_row = adw::SwitchRow::builder()
        .title("Hapus arsip setelah extract")
        .subtitle("Pindahkan arsip ke Trash setelah extract berhasil")
        .build();
    trash_row.set_active(ui.config.borrow().delete_after_extract);
    trash_row.connect_active_notify({
        let ui = ui.clone();
        move |r| {
            let mut c = ui.config.borrow_mut();
            c.delete_after_extract = r.is_active();
            c.save();
        }
    });
    group.add(&trash_row);

    // Tipe berkas terlarang (exclude from extracting).
    let proh_row = adw::EntryRow::builder()
        .title("Tipe berkas dilarang di-extract")
        .build();
    proh_row.set_text(&ui.config.borrow().prohibited.join(" "));
    proh_row.set_tooltip_text(Some(
        "Ekstensi dipisah spasi (mis. \"desktop sh exe\"). Kosong = tanpa filter.",
    ));
    proh_row.connect_apply({
        let ui = ui.clone();
        move |r| {
            let mut c = ui.config.borrow_mut();
            c.prohibited = config::parse_prohibited(&r.text());
            c.save();
        }
    });
    group.add(&proh_row);

    page.add(&group);
    win.add(&page);
    win.present();
}

// ---------------------------------------------------------------------------
// Wizard
// ---------------------------------------------------------------------------

/// Wizard "apa yang ingin Anda lakukan?" — pengganti titik masuk WinRAR,
/// merutekan ke alur yang sudah ada (Planning Doc §5.4).
fn show_wizard(ui: &Rc<Ui>) {
    let win = adw::Window::builder()
        .transient_for(&ui.window)
        .modal(true)
        .title("Wizard Zippy")
        .default_width(440)
        .build();

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("Apa yang ingin Anda lakukan?")
        .build();

    let make_row = |title: &str, subtitle: &str, icon: &str| {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row
    };

    let open = make_row("Buka arsip", "Tampilkan isi arsip yang sudah ada", "document-open");
    open.connect_activated({
        let ui = ui.clone();
        let win = win.clone();
        move |_| {
            win.close();
            open_dialog(&ui);
        }
    });
    let create = make_row("Buat arsip baru", "Pilih berkas/folder lalu kompres", "list-add");
    create.connect_activated({
        let ui = ui.clone();
        let win = win.clone();
        move |_| {
            win.close();
            compress_dialog(&ui);
        }
    });
    let extract = make_row("Extract arsip", "Pilih arsip lalu folder tujuan", "archive-extract");
    extract.connect_activated({
        let ui = ui.clone();
        let win = win.clone();
        move |_| {
            win.close();
            wizard_extract(&ui);
        }
    });
    let test = make_row("Uji arsip", "Verifikasi integritas isi arsip", "dialog-ok-apply");
    test.connect_activated({
        let ui = ui.clone();
        let win = win.clone();
        move |_| {
            win.close();
            wizard_test(&ui);
        }
    });

    group.add(&open);
    group.add(&create);
    group.add(&extract);
    group.add(&test);
    page.add(&group);

    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    win.set_content(Some(&toolbar));
    win.present();
}

/// Wizard: pilih arsip → pilih folder tujuan → extract.
fn wizard_extract(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder().title("Pilih arsip untuk di-extract").build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.open(Some(&win), gio::Cancellable::NONE, move |res| {
        let Ok(file) = res else { return };
        let Some(archive) = file.path() else { return };
        let folder = gtk::FileDialog::builder().title("Extract ke folder…").build();
        let ui = ui.clone();
        folder.select_folder(Some(&ui.window.clone()), gio::Cancellable::NONE, move |res| {
            if let Ok(f) = res {
                if let Some(dest) = f.path() {
                    run_extract(&ui, archive.clone(), dest, None, OverwriteMode::Overwrite);
                }
            }
        });
    });
}

/// Wizard: pilih arsip → uji integritas.
fn wizard_test(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder().title("Pilih arsip untuk diuji").build();
    let win = ui.window.clone();
    let ui = ui.clone();
    dialog.open(Some(&win), gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(archive) = file.path() {
                run_test(&ui, archive, None);
            }
        }
    });
}

fn show_toast(ui: &Ui, msg: &str) {
    ui.toast.add_toast(adw::Toast::new(msg));
}

/// Catat satu baris ber-timestamp ke log sesi (Options → Lihat Log).
fn log_event(ui: &Ui, msg: &str) {
    let ts = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%H:%M:%S").ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    ui.log.borrow_mut().push(format!("[{ts}] {msg}"));
}

/// Options → Lihat Log: tampilkan log operasi sesi dalam dialog scrollable.
fn show_log(ui: &Rc<Ui>) {
    let text = {
        let log = ui.log.borrow();
        if log.is_empty() {
            "(Belum ada aktivitas)".to_string()
        } else {
            log.join("\n")
        }
    };
    let dialog = adw::MessageDialog::new(Some(&ui.window), Some("Log Aktivitas"), None);

    let label = gtk::Label::builder()
        .label(&text)
        .xalign(0.0)
        .selectable(true)
        .wrap(true)
        .build();
    label.add_css_class("monospace");
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(240)
        .min_content_width(420)
        .child(&label)
        .build();
    dialog.set_extra_child(Some(&icon_with("zippy-info", &scroll)));

    dialog.add_response("clear", "Bersihkan");
    dialog.add_response("ok", "Tutup");
    dialog.set_default_response(Some("ok"));
    let ui = ui.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp == "clear" {
            ui.log.borrow_mut().clear();
            show_toast(&ui, "Log dibersihkan");
        }
    });
    dialog.present();
}

/// Pindahkan arsip ke Trash setelah extract sukses; bila itu arsip yang sedang
/// dibuka, tutup tampilannya.
fn trash_archive_after_extract(ui: &Rc<Ui>, archive: &Path) {
    match gio::File::for_path(archive).trash(gio::Cancellable::NONE) {
        Ok(()) => {
            log_event(ui, &format!("Arsip dipindah ke Trash: {}", archive.display()));
            show_toast(ui, "Arsip dipindahkan ke Trash");
            if ui.current.borrow().as_deref() == Some(archive) {
                close_archive(ui);
            }
        }
        Err(e) => show_toast(ui, &format!("Gagal memindah arsip ke Trash: {e}")),
    }
}

/// Toast dengan tombol "Buka Folder" yang membuka `dir` di file manager.
fn show_toast_open_folder(ui: &Rc<Ui>, msg: &str, dir: PathBuf) {
    let toast = adw::Toast::builder().title(msg).button_label("Buka Folder").build();
    let ui2 = ui.clone();
    toast.connect_button_clicked(move |_| {
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&dir)));
        launcher.launch(Some(&ui2.window), gio::Cancellable::NONE, |res| {
            if let Err(e) = res {
                tracing::warn!("buka folder gagal: {e}");
            }
        });
    });
    ui.toast.add_toast(toast);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_header_summary_and_rows() {
        let entries = vec![
            Entry {
                name: "a.txt".into(),
                size: 1000,
                compressed_size: 400,
                is_dir: false,
                modified: Some("2026-06-25 10:00".into()),
                crc32: Some(0xDEADBEEF),
            },
            Entry {
                name: "sub".into(),
                size: 0,
                compressed_size: 0,
                is_dir: true,
                modified: None,
                crc32: None,
            },
        ];
        let r = build_report(Path::new("/tmp/x.zip"), &entries);
        assert!(r.contains("Laporan Archive"));
        assert!(r.contains("/tmp/x.zip"));
        assert!(r.contains("Berkas  : 1"));
        assert!(r.contains("Folder: 1"));
        assert!(r.contains("rasio 40.0%"));
        assert!(r.contains("a.txt\t1000\t400\t2026-06-25 10:00\tDEADBEEF"));
    }
}
