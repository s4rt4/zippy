//! Panel pohon folder (sidebar kiri) gaya WinRAR.
//!
//! Membangun hierarki direktori dari daftar [`Entry`] yang datar, lalu
//! menampilkannya sebagai [`gtk::ListView`] + [`gtk::TreeListModel`]. Mengklik
//! sebuah folder menavigasi daftar utama (lihat penyambungan di `window.rs`).

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use zippy_core::Entry;

// ---------------------------------------------------------------------------
// TreeNode — satu folder dalam pohon (root = komponen kosong)
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TreeNode {
        pub components: RefCell<Vec<String>>,
        pub label: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TreeNode {
        const NAME: &'static str = "ZippyTreeNode";
        type Type = super::TreeNode;
    }

    impl ObjectImpl for TreeNode {}
}

glib::wrapper! {
    pub struct TreeNode(ObjectSubclass<imp::TreeNode>);
}

impl TreeNode {
    fn new(components: Vec<String>, label: String) -> Self {
        let obj: Self = glib::Object::new();
        *obj.imp().components.borrow_mut() = components;
        *obj.imp().label.borrow_mut() = label;
        obj
    }

    /// Komponen path folder (kosong = root archive).
    pub fn components(&self) -> Vec<String> {
        self.imp().components.borrow().clone()
    }

    fn label(&self) -> String {
        self.imp().label.borrow().clone()
    }
}

// ---------------------------------------------------------------------------
// FolderTree
// ---------------------------------------------------------------------------

pub struct FolderTree {
    pub widget: gtk::ScrolledWindow,
    pub list_view: gtk::ListView,
    root: gio::ListStore,
    model: gtk::TreeListModel,
    /// Semua path direktori yang diketahui (dipakai create_func anak).
    dirs: Rc<RefCell<BTreeSet<Vec<String>>>>,
}

pub fn build() -> FolderTree {
    let root = gio::ListStore::new::<TreeNode>();
    let dirs: Rc<RefCell<BTreeSet<Vec<String>>>> = Rc::new(RefCell::new(BTreeSet::new()));

    let dirs_for_children = dirs.clone();
    let model = gtk::TreeListModel::new(root.clone(), false, false, move |item| {
        let node = item.downcast_ref::<TreeNode>()?;
        let comps = node.components();
        let store = gio::ListStore::new::<TreeNode>();
        for path in dirs_for_children.borrow().iter() {
            // Anak langsung: satu komponen lebih dalam & berbagi prefiks.
            if path.len() == comps.len() + 1 && path[..comps.len()] == comps[..] {
                let label = path.last().cloned().unwrap_or_default();
                store.append(&TreeNode::new(path.clone(), label));
            }
        }
        if store.n_items() == 0 {
            None
        } else {
            Some(store.upcast())
        }
    });

    let selection = gtk::SingleSelection::new(Some(model.clone()));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let expander = gtk::TreeExpander::new();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&gtk::Image::from_icon_name("folder"));
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        row.append(&label);
        expander.set_child(Some(&row));
        item.set_child(Some(&expander));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(trow) = item.item().and_downcast::<gtk::TreeListRow>() else {
            return;
        };
        let Some(node) = trow.item().and_downcast::<TreeNode>() else {
            return;
        };
        let expander = item.child().and_downcast::<gtk::TreeExpander>().unwrap();
        expander.set_list_row(Some(&trow));
        let row = expander.child().and_downcast::<gtk::Box>().unwrap();
        let label = row.last_child().and_downcast::<gtk::Label>().unwrap();
        label.set_text(&node.label());
    });

    let list_view = gtk::ListView::new(Some(selection), Some(factory));
    list_view.set_single_click_activate(true);
    list_view.add_css_class("navigation-sidebar");

    let widget = gtk::ScrolledWindow::builder()
        .min_content_width(180)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .child(&list_view)
        .build();

    FolderTree {
        widget,
        list_view,
        root,
        model,
        dirs,
    }
}

impl FolderTree {
    /// Bangun ulang pohon dari `entries`. `archive_label` jadi nama node root.
    pub fn rebuild(&self, entries: &[Entry], archive_label: &str) {
        *self.dirs.borrow_mut() = dir_paths(entries);

        self.root.remove_all();
        self.root
            .append(&TreeNode::new(Vec::new(), archive_label.to_string()));
        // Buka node root agar folder tingkat-atas langsung terlihat.
        if let Some(row) = self.model.item(0).and_downcast::<gtk::TreeListRow>() {
            row.set_expanded(true);
        }
    }

    /// Komponen folder pada baris `pos` (untuk handler activate).
    pub fn components_at(&self, pos: u32) -> Option<Vec<String>> {
        let row = self
            .list_view
            .model()?
            .item(pos)
            .and_downcast::<gtk::TreeListRow>()?;
        let node = row.item().and_downcast::<TreeNode>()?;
        Some(node.components())
    }
}

/// Kumpulan path direktori (komponen) dari daftar entri datar — termasuk folder
/// implisit dari path file bersarang. Tidak menyertakan root (komponen kosong).
pub(crate) fn dir_paths(entries: &[Entry]) -> BTreeSet<Vec<String>> {
    let mut set: BTreeSet<Vec<String>> = BTreeSet::new();
    for e in entries {
        let comps: Vec<String> = e
            .name
            .trim_end_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        // Untuk file, hanya folder induknya yang dianggap direktori.
        let upto = if e.is_dir {
            comps.len()
        } else {
            comps.len().saturating_sub(1)
        };
        for i in 1..=upto {
            set.insert(comps[..i].to_vec());
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.to_string(),
            size: 0,
            compressed_size: 0,
            is_dir,
            modified: None,
            crc32: None,
        }
    }

    #[test]
    fn dir_paths_includes_implicit_parents() {
        let entries = vec![
            entry("a.txt", false),
            entry("sub/b.txt", false),
            entry("sub/deep/c.txt", false),
            entry("empty/", true),
        ];
        let dirs = dir_paths(&entries);
        // File root tidak menambah folder; folder induk file bersarang masuk.
        assert!(dirs.contains(&vec!["sub".to_string()]));
        assert!(dirs.contains(&vec!["sub".to_string(), "deep".to_string()]));
        assert!(dirs.contains(&vec!["empty".to_string()]));
        // "a.txt" bukan folder.
        assert!(!dirs.contains(&vec!["a.txt".to_string()]));
        assert_eq!(dirs.len(), 3);
    }
}
