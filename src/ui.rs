//! Widgets and helpers shared by the components.

use crate::library::Library;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Shared by the components. The app replaces its content on reload
pub type Lib = Rc<RefCell<Library>>;

/// The local time in ISO 8601 with an offset, for `added`
pub fn now() -> String {
    glib::DateTime::now_local().and_then(|d| d.format("%FT%T%:z")).map(|s| s.to_string()).unwrap_or_default()
}

/// The last part of a path, to show; "" if there is none
pub fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// A decoded image, to show
pub fn texture(image: image::RgbaImage) -> gdk::MemoryTexture {
    let (width, height) = (image.width() as i32, image.height() as i32);
    let bytes = glib::Bytes::from_owned(image.into_raw());
    gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::R8g8b8a8, &bytes, width as usize * 4)
}

// MARK: - Actions

/// Calls `f` on a key press that no focused widget used
pub fn shortcut(widget: &impl IsA<gtk::Widget>, trigger: &str, f: impl Fn() -> glib::Propagation + 'static) {
    let action = gtk::CallbackAction::new(move |_, _| f());
    let controller = gtk::ShortcutController::new();
    controller.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string(trigger), Some(action)));
    widget.add_controller(controller);
}

/// Adds an action without a parameter to a window or a group
pub fn action(map: &impl IsA<gio::ActionMap>, name: &str, f: impl Fn() + 'static) -> gio::SimpleAction {
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| f());
    map.add_action(&action);
    action
}

pub fn menu_button(menu: &gio::Menu) -> gtk::MenuButton {
    gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(menu)
        .primary(true)
        .tooltip_text("Main Menu")
        .build()
}

// MARK: - Layout

/// A page's content, which may shrink to the window's minimum size, and holds the page's
/// breakpoints
pub fn bin(content: &impl IsA<gtk::Widget>) -> adw::BreakpointBin {
    adw::BreakpointBin::builder().width_request(360).height_request(300).child(content).build()
}

pub fn boxed_list() -> gtk::ListBox {
    gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build()
}

pub fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(child).build()
}

// MARK: - Search

/// A search entry in a search bar under the header, above the content. The bar opens from a
/// header button, Ctrl+F, or typing
pub struct SearchView {
    pub view: adw::ToolbarView,
    pub bar: gtk::SearchBar,
}

/// `search` gets the text as it changes
pub fn search_view(
    header: &adw::HeaderBar,
    content: &impl IsA<gtk::Widget>,
    search: impl Fn(String) + 'static,
) -> SearchView {
    let entry = gtk::SearchEntry::builder().placeholder_text("Search").build();
    entry.connect_search_changed(move |entry| search(entry.text().into()));
    let clamp = adw::Clamp::builder().maximum_size(400).child(&entry).build();
    let bar = gtk::SearchBar::builder().child(&clamp).build();
    bar.connect_entry(&entry);
    let toggle =
        gtk::ToggleButton::builder().icon_name("system-search-symbolic").tooltip_text("Search").build();
    toggle.bind_property("active", &bar, "search-mode-enabled").bidirectional().build();
    header.pack_end(&toggle);
    // Only while shown: otherwise typing on another page would open this page's search
    bar.connect_map(|bar| bar.set_key_capture_widget(bar.root().as_ref()));
    bar.connect_unmap(|bar| bar.set_key_capture_widget(gtk::Widget::NONE));

    let view = adw::ToolbarView::builder().content(content).build();
    view.add_top_bar(header);
    view.add_top_bar(&bar);
    let b = bar.downgrade();
    shortcut(&view, "<Control>f", move || {
        b.upgrade().iter().for_each(|b| b.set_search_mode(!b.is_search_mode()));
        glib::Propagation::Stop
    });
    SearchView { view, bar }
}

/// Fills `stack` with `content` and a "No Results" page; `show_results` picks one
pub fn add_results(stack: &gtk::Stack, content: &impl IsA<gtk::Widget>) {
    stack.add_named(content, Some("content"));
    let status = adw::StatusPage::builder().icon_name("edit-find-symbolic").title("No Results").build();
    stack.add_named(&status, Some("empty"));
}

pub fn show_results(stack: &gtk::Stack, any: bool) {
    stack.set_visible_child_name(if any { "content" } else { "empty" });
}

// MARK: - Trash

/// Asks whether to move the image and its sidecar to the trash, then does, and calls `done` with
/// the image and the result
pub fn ask_trash(
    parent: &impl IsA<gtk::Widget>,
    lib: &Lib,
    path: PathBuf,
    done: impl Fn(PathBuf, Result<(), String>) + 'static,
) {
    let dialog = adw::AlertDialog::new(
        Some("Move to Trash?"),
        Some(&format!("“{}” and its sidecar will be moved to the trash.", file_name(&path))),
    );
    dialog.add_responses(&[("cancel", "_Cancel"), ("trash", "_Move to Trash")]);
    dialog.set_response_appearance("trash", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let lib = lib.clone();
    dialog.connect_response(Some("trash"), move |_, _| {
        let result = trash(&mut lib.borrow_mut(), &path);
        done(path.clone(), result);
    });
    dialog.present(Some(parent));
}

/// Moves the image and its sidecar to the trash, not deleting them, so that it can be undone in
/// Files, and leaves the image out of the library. Returns what to tell the user
fn trash(lib: &mut Library, path: &Path) -> Result<(), String> {
    // Not the folder itself, which is the file of the empty ID
    if lib.get(path).is_none() {
        return Err("The image is gone".into());
    }
    let trash = |p: &Path| match p.exists() {
        true => gio::File::for_path(p).trash(gio::Cancellable::NONE),
        false => Ok(()),
    };
    let result = trash(&lib.file(path)).and_then(|_| trash(&lib.sidecar(path)));
    // Gone, if only the sidecar is left
    if !lib.file(path).exists() {
        lib.forget(path);
    }
    result.map_err(|e| format!("Could not move to the trash: {}", e.message()))
}

// MARK: - Thumbnails

/// A square thumbnail that fills its tile
pub fn thumbnail() -> gtk::Picture {
    gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .overflow(gtk::Overflow::Hidden)
        .build()
}

/// Makes `grid` a gallery of square tiles: at least as wide as the zoom (in CSS), stretched to
/// fill the row
pub fn tile_grid(grid: &gtk::GridView) {
    grid.set_single_click_activate(true);
    // GTK's default is 7
    grid.set_max_columns(64);
    // Rows as tall as a square thumbnail at the stretched width, not at the minimum
    grid.set_vscroll_policy(gtk::ScrollablePolicy::Natural);
    grid.add_css_class("gallery");
}
