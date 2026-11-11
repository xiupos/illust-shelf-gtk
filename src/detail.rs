//! Detail page. Built once when opened; edits are saved once, when the page is hidden
//! or the window closes (main.rs activates `detail.save` then).

use crate::model::{Item, Library};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Saves the page's edits. Does nothing if nothing changed
pub const SAVE: &str = "detail.save";
/// Edited fields, in the order of the editors
const FIELDS: [&str; 4] = ["title", "tags", "urls", "note"];

thread_local! {
    /// Whether the details pane is shown, kept from page to page like GNOME's Image Viewer does
    static SHOW_DETAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// `tag` ("" for all) and `query` describe the gallery the page was opened from; the previous and
/// next buttons (and the Left / Right keys) step through that gallery's order
pub fn page(lib: &Rc<RefCell<Library>>, path: &Path, tag: &str, query: &str) -> adw::NavigationPage {
    // Pick up changes made in a text editor since the library was loaded
    lib.borrow_mut().refresh(path);
    let item = lib.borrow().get(path).cloned().unwrap_or_else(|| Item {
        path: path.to_owned(),
        meta: serde_json::json!({"v": 1}),
        editable: false,
    });
    let editable = item.editable;

    let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).can_shrink(true).build();
    picture.set_paintable(texture(path).as_ref());

    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    let title = adw::EntryRow::builder().title("Title").text(item.text("title")).sensitive(editable).build();
    panel.append(&group(None, &title));

    // Tag rows open the tag's gallery; URL rows open the browser
    let (tags_group, tags) = list_editor("Tags", "Add Tag", item.tags(), editable, |row, tag| {
        row.set_action_name(Some("win.show-scope"));
        row.set_action_target_value(Some(&tag.to_variant()));
    });
    panel.append(&tags_group);
    let (urls_group, urls) = list_editor("Sources", "Add URL", item.urls(), editable, |row, _| {
        row.set_title_lines(1);
        row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
        row.connect_activated(|row| {
            let window = row.root().and_downcast::<gtk::Window>();
            let w = window.clone();
            gtk::UriLauncher::new(&row.title()).launch(window.as_ref(), gio::Cancellable::NONE, move |r| {
                if let (Err(e), Some(w)) = (r, w) {
                    toast(&w, &format!("Could not open: {}", e.message()));
                }
            });
        });
    });
    panel.append(&urls_group);

    let note = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .editable(editable)
        .top_margin(10)
        .bottom_margin(10)
        .left_margin(10)
        .right_margin(10)
        .height_request(100)
        .build();
    note.buffer().set_text(&item.text("note"));
    panel.append(&group(Some("Note"), &gtk::Frame::builder().child(&note).css_classes(["view"]).build()));

    // As in GNOME's Image Viewer's properties: the folder with a button to open it, then the path
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let property = |title: &str, value: &str| {
        adw::ActionRow::builder()
            .title(title)
            .subtitle(value)
            .subtitle_selectable(true)
            .subtitle_lines(1)
            .use_markup(false)
            .css_classes(["property"])
            .build()
    };
    let parent = path.parent().unwrap_or(path);
    let folder = property("Folder", &parent.file_name().unwrap_or_default().to_string_lossy());
    let open = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Open Containing Folder")
        .valign(gtk::Align::Center)
        .action_name("detail.open-folder")
        .css_classes(["flat"])
        .build();
    folder.add_suffix(&open);
    folder.set_activatable_widget(Some(&open));
    let file = adw::PreferencesGroup::new();
    file.add(&folder);
    file.add(&property("Path", &path.to_string_lossy()));
    panel.append(&file);

    let near = siblings(lib, path, tag, query);
    // Opens the image, its folder, or the sidecar in other apps (GNOME's image viewer handles
    // zooming, so the app does not)
    let menu = gio::Menu::new();
    menu.append(Some("Open in Image Viewer"), Some("detail.open-image"));
    menu.append(Some("Open Containing Folder"), Some("detail.open-folder"));
    menu.append(Some("Edit Sidecar in Text Editor"), Some("detail.edit-sidecar"));
    let more = menu_button(&menu);
    let (image, poke) = image_with_controls(&picture, &near, &more);

    // Layout of GNOME's Image Viewer: the details pane on the right, or below the image when
    // narrow, both shown by the (i) button or F9
    let toggle = gtk::ToggleButton::builder()
        .icon_name("help-about-symbolic")
        .tooltip_text("Details")
        .active(SHOW_DETAILS.get())
        .build();
    toggle.connect_toggled(|t| SHOW_DETAILS.set(t.is_active()));
    let sidebar = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(316)
        .child(&panel)
        .build();
    let split = adw::OverlaySplitView::builder()
        .sidebar_position(gtk::PackType::End)
        .min_sidebar_width(255.0)
        .content(&adw::LayoutSlot::new("content"))
        .sidebar(&adw::LayoutSlot::new("details"))
        .build();
    let bottom = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .child(&adw::LayoutSlot::new("details"))
        .css_classes(["bottom-details"])
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let slot = adw::LayoutSlot::builder().id("content").vexpand(true).build();
    column.append(&slot);
    column.append(&bottom);
    let layouts = adw::MultiLayoutView::new();
    for (name, content) in [("wide", split.upcast_ref::<gtk::Widget>()), ("narrow", column.upcast_ref())] {
        let layout = adw::Layout::new(content);
        layout.set_name(Some(name));
        layouts.add_layout(layout);
    }
    layouts.set_child("content", &image);
    layouts.set_child("details", &sidebar);
    layouts.set_layout_name("wide");
    toggle.bind_property("active", &split, "show-sidebar").bidirectional().sync_create().build();
    toggle.bind_property("active", &bottom, "reveal-child").bidirectional().sync_create().build();
    let header = adw::HeaderBar::new();
    header.pack_start(&fullscreen_button());
    header.pack_end(&more);
    header.pack_end(&toggle);
    // The border variant of raised, to avoid a shadow over the image; the read-only banner sits
    // below the header
    let view =
        adw::ToolbarView::builder().content(&layouts).top_bar_style(adw::ToolbarStyle::RaisedBorder).build();
    view.add_top_bar(&header);
    if !editable {
        let banner = adw::Banner::new("Read-only: the sidecar is unreadable or has an unsupported version");
        banner.set_revealed(true);
        view.add_top_bar(&banner);
    }
    let t = toggle.clone();
    let f9 = gtk::CallbackAction::new(move |_, _| {
        t.set_active(!t.is_active());
        glib::Propagation::Stop
    });
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("F9"), Some(f9)));
    // F11 toggles fullscreen; Escape leaves it, and goes back only when not fullscreen
    let f11 = gtk::CallbackAction::new(|page, _| {
        toggle_fullscreen(page);
        glib::Propagation::Stop
    });
    shortcuts.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("F11"), Some(f11)));
    let escape = gtk::CallbackAction::new(|page, _| match page.root().and_downcast::<gtk::Window>() {
        Some(window) if window.is_fullscreen() => {
            window.unfullscreen();
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    shortcuts.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("Escape"), Some(escape)));
    // Left / Right reach this only when no focused widget (a text field, a list) used them
    for (key, target) in near {
        let shortcut = gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(key),
            Some(gtk::NamedAction::new("win.show-sibling")),
        );
        shortcut.set_arguments(target.as_ref());
        shortcuts.add_shortcut(shortcut);
    }
    view.add_controller(shortcuts);
    let bin = adw::BreakpointBin::builder().width_request(360).height_request(300).child(&view).build();
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 590sp").unwrap());
    narrow.add_setter(&layouts, "layout-name", Some(&"narrow".to_value()));
    bin.add_breakpoint(narrow);
    // Pointer motion anywhere on the page shows the image controls. GTK also reports motion
    // without a move when widgets change under a still pointer, so only real moves count
    let motion = gtk::EventControllerMotion::new();
    let (p, last) = (poke.clone(), std::cell::Cell::new((-1.0, -1.0)));
    motion.connect_motion(move |_, x, y| {
        if last.replace((x, y)) != (x, y) {
            p();
        }
    });
    bin.add_controller(motion);
    let page = adw::NavigationPage::new(&bin, &name);

    // Saving: compare each field with what was last saved and write only the changed ones,
    // on top of the sidecar re-read from disk (so text editor changes to other fields survive)
    let texts = move || {
        let buffer = note.buffer();
        let note = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
        [title.text().to_string(), tags(), urls(), note]
    };
    let saved = RefCell::new(texts());
    let window = glib::WeakRef::<gtk::Window>::new();
    let w = window.clone();
    page.connect_map(move |page| w.set(page.root().and_downcast::<gtk::Window>().as_ref()));
    let (l, image, w, sidecar) = (lib.clone(), path.to_owned(), window.clone(), item.sidecar_path());
    let save = gio::SimpleAction::new("save", None);
    let (lib, path) = (lib.clone(), path.to_owned());
    save.connect_activate(move |_, _| {
        let current = texts();
        let changed: Vec<(&str, &String)> = FIELDS
            .into_iter()
            .zip(&current)
            .zip(saved.borrow().iter())
            .filter(|((_, now), before)| now != before)
            .map(|(field, _)| field)
            .collect();
        if changed.is_empty() {
            return;
        }
        let result = lib.borrow_mut().edit(&path, &now(), |it| {
            for (key, text) in &changed {
                it.set_text(key, text);
            }
        });
        match result {
            Ok(()) => *saved.borrow_mut() = current.clone(),
            Err(e) => {
                if let Some(w) = window.upgrade() {
                    toast(&w, &format!("Could not save: {e}"));
                }
            }
        }
    });
    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&save);
    for (name, folder) in [("open-image", false), ("open-folder", true)] {
        let action = gio::SimpleAction::new(name, None);
        let (image, w) = (image.clone(), w.clone());
        action.connect_activate(move |_, _| launch(&w, &image, folder));
        actions.add_action(&action);
    }
    let edit = gio::SimpleAction::new("edit-sidecar", None);
    let s = save.clone();
    edit.connect_activate(move |_, _| {
        // Save pending edits first, and create {"v":1,"added":…} if there is no sidecar
        s.activate(None);
        let result = l.borrow_mut().create(&image, &now());
        if let (Err(e), Some(win)) = (result, w.upgrade()) {
            toast(&win, &format!("Could not create the sidecar: {e}"));
        }
        launch(&w, &sidecar, false);
    });
    actions.add_action(&edit);
    page.insert_action_group("detail", Some(&actions));
    page.connect_hidden(|page| {
        let _ = page.activate_action(SAVE, None);
    });

    // By default the title field gets focus with its text selected, and a keystroke would
    // replace it
    page.connect_shown(move |_| {
        toggle.grab_focus();
        poke();
    });
    page
}

/// For the previous and next images: (key, `win.show-sibling` target, if there is one)
fn siblings(
    lib: &Rc<RefCell<Library>>,
    path: &Path,
    tag: &str,
    query: &str,
) -> [(&'static str, Option<glib::Variant>); 2] {
    let lib = lib.borrow();
    let paths = lib.paths(Some(tag).filter(|t| !t.is_empty()), query);
    let i = paths.iter().position(|p| Path::new(p) == path);
    let at = |i: Option<usize>| i.and_then(|i| paths.get(i)).map(|p| (*p, tag, query).to_variant());
    [("Left", at(i.and_then(|i| i.checked_sub(1)))), ("Right", at(i.map(|i| i + 1)))]
}

/// The image with previous / next buttons at its bottom left, as in GNOME's Image Viewer (whose
/// style and timings these copy). The returned closure shows them; they fade out after 3 s
/// unless the pointer is on them or the menu is open
fn image_with_controls(
    picture: &gtk::Picture,
    near: &[(&str, Option<glib::Variant>); 2],
    menu: &gtk::MenuButton,
) -> (gtk::Overlay, Rc<dyn Fn()>) {
    let controls = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::End)
        .css_classes(["osd-box"])
        .build();
    for ((icon, tooltip), (_, target)) in
        [("go-previous-symbolic", "Previous Image"), ("go-next-symbolic", "Next Image")].into_iter().zip(near)
    {
        let button = gtk::Button::builder()
            .icon_name(icon)
            .tooltip_text(tooltip)
            .sensitive(target.is_some())
            .css_classes(["flat"])
            .build();
        if let Some(target) = target {
            button.set_action_name(Some("win.show-sibling"));
            button.set_action_target_value(Some(target));
        }
        controls.append(&button);
    }
    let hovered = gtk::EventControllerMotion::new();
    controls.add_controller(hovered.clone());
    let overlay = gtk::Overlay::builder().child(picture).build();
    overlay.add_overlay(&controls);

    let fade = |to: f64, ms: u32| {
        let c = controls.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| c.set_opacity(v));
        adw::TimedAnimation::builder().widget(&controls).duration(ms).value_to(to).target(&target).build()
    };
    let (show, hide) = (fade(1.0, 200), fade(0.0, 1000));
    let animate = move |run: &adw::TimedAnimation, stop: &adw::TimedAnimation| {
        let from = controls.opacity();
        if run.state() != adw::AnimationState::Playing && from != run.value_to() {
            stop.pause();
            run.set_value_from(from);
            run.play();
        }
    };
    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let menu = menu.downgrade();
    let poke: Rc<dyn Fn()> = Rc::new(move || {
        animate(&show, &hide);
        if let Some(id) = timer.take() {
            id.remove();
        }
        if hovered.contains_pointer() || menu.upgrade().is_some_and(|m| m.is_active()) {
            return;
        }
        let (t, animate, show, hide) = (timer.clone(), animate.clone(), show.clone(), hide.clone());
        *timer.borrow_mut() =
            Some(glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                t.take();
                animate(&hide, &show);
            }));
    });
    // Clicks on the image show them too
    let click = gtk::GestureClick::new();
    let p = poke.clone();
    click.connect_pressed(move |_, _, _, _| p());
    overlay.add_controller(click);
    (overlay, poke)
}

/// The window's fullscreen toggle, as in GNOME's Image Viewer. Its icon follows the window while
/// the page is shown
fn fullscreen_button() -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("view-fullscreen-symbolic")
        .tooltip_text("Toggle Fullscreen")
        .build();
    button.connect_clicked(toggle_fullscreen);
    let binding: Rc<RefCell<Option<glib::Binding>>> = Rc::default();
    let b = binding.clone();
    button.connect_map(move |button| {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else { return };
        let icon = |_: &glib::Binding, full: bool| {
            Some(if full { "view-restore-symbolic" } else { "view-fullscreen-symbolic" })
        };
        b.replace(Some(
            window
                .bind_property("fullscreened", button, "icon-name")
                .transform_to(icon)
                .sync_create()
                .build(),
        ));
    });
    button.connect_unmap(move |_| {
        if let Some(b) = binding.take() {
            b.unbind();
        }
    });
    button
}

fn toggle_fullscreen(widget: &impl IsA<gtk::Widget>) {
    if let Some(window) = widget.root().and_downcast::<gtk::Window>() {
        window.set_fullscreened(!window.is_fullscreen());
    }
}

/// The hamburger button used by every page (HIG: `open-menu-symbolic`, "Main Menu", F10)
pub fn menu_button(menu: &gio::Menu) -> gtk::MenuButton {
    gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(menu)
        .primary(true)
        .tooltip_text("Main Menu")
        .build()
}

/// The full image. GTK reads PNG, JPEG, and TIFF itself and the rest through gdk-pixbuf loaders,
/// which may be missing (WebP needs webp-pixbuf-loader), so fall back to the `image` crate that
/// made the thumbnail
fn texture(path: &Path) -> Option<gdk::Texture> {
    gdk::Texture::from_filename(path).ok().or_else(|| {
        let image = image::open(path).ok()?.into_rgba8();
        let (w, h) = image.dimensions();
        let bytes = glib::Bytes::from_owned(image.into_raw());
        let texture =
            gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, w as usize * 4);
        Some(texture.upcast())
    })
}

/// Current local time in ISO 8601 with an offset, for `added`
fn now() -> String {
    glib::DateTime::now_local().and_then(|d| d.format("%FT%T%:z")).map(|s| s.to_string()).unwrap_or_default()
}

fn group(title: Option<&str>, child: &impl IsA<gtk::Widget>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    if let Some(title) = title {
        group.set_title(title);
    }
    group.add(child);
    group
}

/// A boxed list with one removable row per value and a last row to add one. The rows hold the
/// state: the returned closure gives the values one per line, including unsubmitted input
fn list_editor(
    title: &str,
    add: &str,
    values: Vec<&str>,
    editable: bool,
    setup: fn(&adw::ActionRow, &str),
) -> (adw::PreferencesGroup, impl Fn() -> String + use<>) {
    let list =
        gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
    for value in values {
        list.append(&value_row(value, editable, setup));
    }
    let entry = adw::EntryRow::builder().title(add).show_apply_button(true).sensitive(editable).build();
    entry.connect_apply(move |entry| {
        let text = entry.text().trim().to_owned();
        let list = entry.parent().and_downcast::<gtk::ListBox>().unwrap();
        if !text.is_empty() && !rows(&list).contains(&text) {
            list.insert(&value_row(&text, editable, setup), entry.index());
        }
        entry.set_text("");
    });
    list.append(&entry);
    let (l, e) = (list.clone(), entry.clone());
    let text = move || [rows(&l), vec![e.text().to_string()]].concat().join("\n");
    (group(Some(title), &list), text)
}

fn value_row(value: &str, editable: bool, setup: fn(&adw::ActionRow, &str)) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(value).use_markup(false).activatable(true).build();
    setup(&row, value);
    let remove = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text("Remove")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .sensitive(editable)
        .build();
    // Found through the widget tree rather than captured, so the button does not hold its own row.
    // Focus then goes to the add row, since the removed row may have had it
    remove.connect_clicked(|button| {
        let row = button.ancestor(adw::ActionRow::static_type()).and_downcast::<adw::ActionRow>().unwrap();
        let list = row.parent().and_downcast::<gtk::ListBox>().unwrap();
        list.remove(&row);
        if let Some(entry) = list.last_child() {
            entry.grab_focus();
        }
    });
    row.add_suffix(&remove);
    row
}

/// The values of a list editor's rows
fn rows(list: &gtk::ListBox) -> Vec<String> {
    let mut values = Vec::new();
    let mut child = list.first_child();
    while let Some(c) = child {
        if let Some(row) = c.downcast_ref::<adw::ActionRow>() {
            values.push(row.title().to_string());
        }
        child = c.next_sibling();
    }
    values
}

/// Opens `file` in its default app, or its folder in the file manager with it selected
fn launch(window: &glib::WeakRef<gtk::Window>, file: &Path, folder: bool) {
    let window = window.upgrade();
    let w = window.clone();
    let done = move |result: Result<(), glib::Error>| {
        if let (Err(e), Some(w)) = (result, w) {
            toast(&w, &format!("Could not open: {}", e.message()));
        }
    };
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(file)));
    if folder {
        launcher.open_containing_folder(window.as_ref(), gio::Cancellable::NONE, done);
    } else {
        launcher.launch(window.as_ref(), gio::Cancellable::NONE, done);
    }
}

fn toast(w: &impl IsA<gtk::Widget>, text: &str) {
    let _ = w.activate_action("win.toast", Some(&text.to_variant()));
}
