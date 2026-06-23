//! Archive content view (GtkColumnView) — gaya WinRAR.
//!
//! Kolom: Nama | Ukuran | Packed | Tipe | Modified | CRC32 (Planning Doc §5.1).
//! Tiap baris adalah [`EntryObject`] (subclass GObject) di dalam `gio::ListStore`.
//! Baris bisa berupa entry asli, sub-folder (untuk navigasi), atau ".." (naik).

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use std::cell::{Cell, RefCell};

// ---------------------------------------------------------------------------
// EntryObject — satu baris
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct EntryObject {
        pub name: RefCell<String>,
        pub full_path: RefCell<String>,
        pub size: Cell<u64>,
        pub packed: Cell<u64>,
        pub is_dir: Cell<bool>,
        pub is_parent: Cell<bool>,
        pub modified: RefCell<String>,
        pub crc: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EntryObject {
        const NAME: &'static str = "ZippyEntryObject";
        type Type = super::EntryObject;
    }

    impl ObjectImpl for EntryObject {}
}

glib::wrapper! {
    pub struct EntryObject(ObjectSubclass<imp::EntryObject>);
}

/// Data satu baris yang sudah dihitung untuk ditampilkan.
pub struct Row {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub is_parent: bool,
    pub size: u64,
    pub packed: u64,
    pub modified: String,
    pub crc: Option<u32>,
}

impl EntryObject {
    pub fn from_row(r: &Row) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        *imp.name.borrow_mut() = r.name.clone();
        *imp.full_path.borrow_mut() = r.full_path.clone();
        imp.size.set(r.size);
        imp.packed.set(r.packed);
        imp.is_dir.set(r.is_dir);
        imp.is_parent.set(r.is_parent);
        *imp.modified.borrow_mut() = r.modified.clone();
        *imp.crc.borrow_mut() = r.crc.map(|c| format!("{c:08X}")).unwrap_or_default();
        obj
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }
    pub fn full_path(&self) -> String {
        self.imp().full_path.borrow().clone()
    }
    pub fn is_dir(&self) -> bool {
        self.imp().is_dir.get()
    }
    pub fn is_parent(&self) -> bool {
        self.imp().is_parent.get()
    }
    fn size(&self) -> u64 {
        self.imp().size.get()
    }
    fn packed(&self) -> u64 {
        self.imp().packed.get()
    }
    fn modified(&self) -> String {
        self.imp().modified.borrow().clone()
    }
    fn crc(&self) -> String {
        self.imp().crc.borrow().clone()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub struct FileListView {
    pub widget: gtk::ScrolledWindow,
    pub column_view: gtk::ColumnView,
    pub store: gio::ListStore,
}

/// Nama ikon tema untuk satu baris.
fn icon_for(e: &EntryObject) -> &'static str {
    if e.is_parent() || e.is_dir() {
        "folder"
    } else {
        "text-x-generic"
    }
}

pub fn build() -> FileListView {
    let store = gio::ListStore::new::<EntryObject>();
    let selection = gtk::MultiSelection::new(Some(store.clone()));

    let column_view = gtk::ColumnView::builder()
        .model(&selection)
        .show_column_separators(true)
        .show_row_separators(true)
        .single_click_activate(false)
        .build();

    // Kolom Nama: ikon + label.
    column_view.append_column(&name_column());
    column_view.append_column(&text_column("Ukuran", true, false, |e| {
        size_cell(e.size(), e)
    }));
    column_view.append_column(&text_column("Packed", true, false, |e| {
        size_cell(e.packed(), e)
    }));
    column_view.append_column(&text_column("Tipe", false, false, |e| {
        type_label(&e.name(), e.is_dir() || e.is_parent())
    }));
    column_view.append_column(&text_column("Modified", false, false, |e| e.modified()));
    column_view.append_column(&text_column("CRC32", false, true, |e| e.crc()));

    let widget = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .child(&column_view)
        .build();

    FileListView {
        widget,
        column_view,
        store,
    }
}

/// Sel ukuran: kosong untuk folder / "..".
fn size_cell(bytes: u64, e: &EntryObject) -> String {
    if e.is_dir() || e.is_parent() {
        String::new()
    } else {
        group_thousands(bytes)
    }
}

fn name_column() -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let image = gtk::Image::new();
        let label = gtk::Label::builder().xalign(0.0).build();
        row.append(&image);
        row.append(&label);
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let entry = item.item().and_downcast::<EntryObject>().unwrap();
        let row = item.child().and_downcast::<gtk::Box>().unwrap();
        let image = row.first_child().and_downcast::<gtk::Image>().unwrap();
        let label = row.last_child().and_downcast::<gtk::Label>().unwrap();
        image.set_icon_name(Some(icon_for(&entry)));
        label.set_text(&entry.name());
    });

    gtk::ColumnViewColumn::builder()
        .title("Nama")
        .factory(&factory)
        .expand(true)
        .resizable(true)
        .build()
}

fn text_column<F>(title: &str, numeric: bool, fixed: bool, value: F) -> gtk::ColumnViewColumn
where
    F: Fn(&EntryObject) -> String + 'static,
{
    let factory = gtk::SignalListItemFactory::new();
    let xalign = if numeric { 1.0 } else { 0.0 };

    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(xalign).build();
        if numeric {
            label.add_css_class("numeric");
        }
        item.set_child(Some(&label));
    });
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let entry = item.item().and_downcast::<EntryObject>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        label.set_text(&value(&entry));
    });

    let mut col = gtk::ColumnViewColumn::builder()
        .title(title)
        .factory(&factory)
        .resizable(true);
    if fixed {
        col = col.fixed_width(96);
    }
    col.build()
}

// ---------------------------------------------------------------------------
// util format
// ---------------------------------------------------------------------------

/// Angka byte dengan pemisah ribuan (gaya WinRAR: "7,949,824").
pub fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c as char);
    }
    out
}

/// Label tipe gaya WinRAR dari ekstensi.
fn type_label(name: &str, is_dir: bool) -> String {
    if is_dir {
        return "Folder".to_string();
    }
    let ext = name.rsplit('.').next().filter(|e| *e != name).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "exe" | "dll" | "so" | "bin" => "Application".to_string(),
        "txt" | "md" | "log" => "Text Document".to_string(),
        "html" | "htm" => "File HTML".to_string(),
        "" => "File".to_string(),
        other => format!("File {}", other.to_uppercase()),
    }
}
