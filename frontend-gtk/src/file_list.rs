//! Archive content view (GtkColumnView).
//!
//! Menampilkan isi archive: Nama | Ukuran | Kompresi | Tipe (Planning Doc §5.1).
//! GtkColumnView dipilih karena lebih performant dari GtkTreeView untuk list
//! panjang (§5.2) — ia hanya merealisasi baris yang terlihat.
//!
//! Tiap [`zippy_core::Entry`] dibungkus jadi [`EntryObject`] (subclass GObject)
//! agar bisa masuk ke `gio::ListStore` yang menjadi model ColumnView.

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use std::cell::{Cell, RefCell};

// ---------------------------------------------------------------------------
// EntryObject — pembungkus GObject untuk satu baris
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct EntryObject {
        pub name: RefCell<String>,
        pub size: Cell<u64>,
        pub compressed: Cell<u64>,
        pub is_dir: Cell<bool>,
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

impl EntryObject {
    pub fn from_entry(e: &zippy_core::Entry) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        *imp.name.borrow_mut() = e.name.clone();
        imp.size.set(e.size);
        imp.compressed.set(e.compressed_size);
        imp.is_dir.set(e.is_dir);
        obj
    }

    fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }
    fn size(&self) -> u64 {
        self.imp().size.get()
    }
    fn compressed(&self) -> u64 {
        self.imp().compressed.get()
    }
    fn is_dir(&self) -> bool {
        self.imp().is_dir.get()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// ColumnView beserta store-nya. `store` dipakai window untuk mengisi/mengosongi
/// daftar saat archive dibuka.
pub struct FileListView {
    pub widget: gtk::ScrolledWindow,
    pub store: gio::ListStore,
}

/// Bangun ColumnView kosong (Nama | Ukuran | Kompresi | Tipe).
pub fn build() -> FileListView {
    let store = gio::ListStore::new::<EntryObject>();
    let selection = gtk::SingleSelection::new(Some(store.clone()));

    let column_view = gtk::ColumnView::builder()
        .model(&selection)
        .show_column_separators(true)
        .show_row_separators(true)
        .build();

    column_view.append_column(&text_column("Nama", true, |e| e.name()));
    column_view.append_column(&text_column("Ukuran", false, |e| human_size(e.size())));
    column_view.append_column(&text_column("Kompresi", false, |e| human_size(e.compressed())));
    column_view.append_column(&text_column("Tipe", false, |e| {
        if e.is_dir() { "Folder".into() } else { "Berkas".into() }
    }));

    let widget = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .child(&column_view)
        .build();

    FileListView { widget, store }
}

/// Kolom teks dengan factory: setup membuat `Label`, bind mengisi teksnya dari
/// `EntryObject` lewat `value`.
fn text_column<F>(title: &str, expand: bool, value: F) -> gtk::ColumnViewColumn
where
    F: Fn(&EntryObject) -> String + 'static,
{
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(0.0).build();
        item.set_child(Some(&label));
    });

    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let entry = item.item().and_downcast::<EntryObject>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        label.set_text(&value(&entry));
    });

    gtk::ColumnViewColumn::builder()
        .title(title)
        .factory(&factory)
        .expand(expand)
        .resizable(true)
        .build()
}

/// Format ukuran byte → human-readable (1 desimal di atas KiB).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}
