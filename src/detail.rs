//! One image, and the pane that edits its sidecar. Every edit is saved at once.

use crate::library::{self, Meta};
use crate::ui::{self, Lib};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use relm4::prelude::*;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// MARK: - Detail

pub struct Detail {
    lib: Lib,
    /// The gallery the page was opened from, as IDs, which previous / next step through
    paths: Vec<PathBuf>,
    index: usize,
    meta: Meta,
    editable: bool,
    /// Every tag in use, most used first
    known: Vec<String>,
}

pub struct Widgets {
    picture: gtk::Picture,
    banner: adw::Banner,
    /// Previous and next
    steps: [gtk::Button; 2],
    /// Shows the pane
    details: gtk::ToggleButton,
    title: adw::EntryRow,
    note: gtk::TextView,
    tags: List,
    urls: List,
    suggestions: gtk::ListBox,
    folder: adw::ActionRow,
    path: adw::ActionRow,
    /// Set while the fields are filled in from code, which is not an edit
    loading: Rc<Cell<bool>>,
}

/// The rows of the tags or the URLs, and the row that adds one
struct List {
    rows: gtk::ListBox,
    entry: adw::EntryRow,
}

#[derive(Debug, Clone, Copy)]
pub enum Field {
    Tags,
    Urls,
}

/// What a row's menu does. Clicking the row opens it
#[derive(Debug, Clone, Copy)]
pub enum Act {
    /// A tag's gallery, or a URL in the browser
    Open,
    Copy,
    Remove,
}

/// What the page's menu opens in other apps
#[derive(Debug, Clone, Copy)]
pub enum Target {
    Image,
    Folder,
    Sidecar,
}

#[derive(Debug)]
pub enum Msg {
    /// Shows the image at the position in the gallery
    Show(Vec<PathBuf>, usize),
    /// To the previous or next image
    Step(isize),
    ToggleDetails,
    Title(String),
    Note(String),
    Add(Field, String),
    /// On the row at the position
    Row(Field, Act, usize),
    /// The text of the row that adds a tag, to suggest tags for
    Typed(String),
    Launch(Target),
    /// Asks, then `Trashed`
    Trash,
    Trashed,
}

#[derive(Debug)]
pub enum Output {
    ShowTag(String),
    TagsChanged,
    Toast(String),
    /// The image is gone
    Trashed,
}

#[derive(Debug)]
pub struct Decoded(PathBuf, Option<image::RgbaImage>);

impl Detail {
    fn path(&self) -> &Path {
        self.paths.get(self.index).map_or(Path::new(""), |p| p)
    }

    fn values(&mut self, field: Field) -> &mut Vec<String> {
        match field {
            Field::Tags => &mut self.meta.tags,
            Field::Urls => &mut self.meta.urls,
        }
    }

    /// Shows the current image and its sidecar, as it is on disk now
    fn load(&mut self, w: &Widgets, sender: &ComponentSender<Self>, page: &adw::NavigationPage) {
        let path = self.path().to_owned();
        let mut lib = self.lib.borrow_mut();
        lib.refresh(&path);
        let file = lib.file(&path);
        let item = lib.get(&path);
        self.meta = item.map(|item| item.meta.clone()).unwrap_or_default();
        self.editable = item.is_some_and(|item| item.editable);
        self.known = lib.tags().into_iter().map(|tag| tag.name).collect();
        drop(lib);

        let name = |p: Option<&Path>| {
            p.and_then(|p| p.file_name()).unwrap_or_default().to_string_lossy().into_owned()
        };
        page.set_title(&name(Some(&file)));
        w.folder.set_subtitle(&name(file.parent()));
        w.path.set_subtitle(&file.to_string_lossy());
        w.banner.set_revealed(!self.editable);
        w.steps[0].set_sensitive(self.index > 0);
        w.steps[1].set_sensitive(self.index + 1 < self.paths.len());
        w.loading.set(true);
        w.title.set_text(&self.meta.title);
        w.note.buffer().set_text(&self.meta.note);
        w.loading.set(false);
        w.title.set_sensitive(self.editable);
        w.note.set_editable(self.editable);
        for field in [Field::Tags, Field::Urls] {
            w.list(field).entry.set_text("");
            w.list(field).entry.set_sensitive(self.editable);
            self.fill(w, field, sender);
        }
        fill(&w.suggestions, &[], |_, tag| suggestion_row(tag, sender));
        // By content, like the thumbnail. GTK itself needs a loader for some formats
        sender.spawn_oneshot_command(move || {
            let image = image::ImageReader::open(&file).and_then(|r| r.with_guessed_format());
            Decoded(path, image.ok().and_then(|r| r.decode().ok()).map(|i| i.into_rgba8()))
        });
    }

    fn fill(&mut self, w: &Widgets, field: Field, sender: &ComponentSender<Self>) {
        let editable = self.editable;
        fill(&w.list(field).rows, self.values(field), |i, value| {
            value_row(field, i, value, editable, sender)
        });
    }

    /// After a change to the tags or URLs
    fn changed(&mut self, w: &Widgets, field: Field, sender: &ComponentSender<Self>) {
        self.save(sender);
        self.fill(w, field, sender);
        let _ = sender.output(Output::TagsChanged);
    }

    fn save(&self, sender: &ComponentSender<Self>) {
        if let Err(e) = self.lib.borrow_mut().save(self.path(), self.meta.clone(), &ui::now()) {
            let _ = sender.output(Output::Toast(format!("Could not save: {e}")));
        }
    }
}

impl Widgets {
    fn list(&self, field: Field) -> &List {
        match field {
            Field::Tags => &self.tags,
            Field::Urls => &self.urls,
        }
    }
}

impl Component for Detail {
    type Init = Lib;
    type Input = Msg;
    type Output = Output;
    type CommandOutput = Decoded;
    type Root = adw::NavigationPage;
    type Widgets = Widgets;

    fn init_root() -> adw::NavigationPage {
        adw::NavigationPage::builder().title("Illust Shelf").build()
    }

    fn init(lib: Lib, page: adw::NavigationPage, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let picture = gtk::Picture::builder().can_shrink(true).build();
        let controls = gtk::Box::builder()
            .spacing(6)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::End)
            .css_classes(["osd", "image-controls"])
            .build();
        let steps = [("go-previous-symbolic", "Previous Image", -1), ("go-next-symbolic", "Next Image", 1)];
        let steps = steps.map(|(icon, tooltip, by)| {
            let button =
                gtk::Button::builder().icon_name(icon).tooltip_text(tooltip).css_classes(["flat"]).build();
            let s = sender.clone();
            button.connect_clicked(move |_| s.input(Msg::Step(by)));
            controls.append(&button);
            button
        });
        let image = gtk::Overlay::builder().child(&picture).css_classes(["image"]).build();
        image.add_overlay(&controls);

        // The pane. Edits made by `load` are not the user's
        let loading = Rc::<Cell<bool>>::default();
        let pane = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(24)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        let title = adw::EntryRow::builder().title("Title").build();
        let (s, l) = (sender.clone(), loading.clone());
        title.connect_changed(move |title| {
            if !l.get() {
                s.input(Msg::Title(title.text().into()));
            }
        });
        pane.append(&group("", &title));
        let suggestions = ui::boxed_list();
        let tags = list(&pane, "Tags", "Add Tag", Field::Tags, Some(&suggestions), &sender);
        let s = sender.clone();
        tags.entry.connect_changed(move |entry| s.input(Msg::Typed(entry.text().into())));
        let urls = list(&pane, "Sources", "Add URL", Field::Urls, None, &sender);
        let note = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(10)
            .bottom_margin(10)
            .left_margin(10)
            .right_margin(10)
            .height_request(100)
            .build();
        let (s, l) = (sender.clone(), loading.clone());
        note.buffer().connect_changed(move |note| {
            if !l.get() {
                s.input(Msg::Note(note.text(&note.start_iter(), &note.end_iter(), false).into()));
            }
        });
        pane.append(&group("Note", &gtk::Frame::builder().child(&note).css_classes(["view"]).build()));
        let property = |title: &str| {
            adw::ActionRow::builder()
                .title(title)
                .subtitle_selectable(true)
                .subtitle_lines(1)
                .use_markup(false)
                .css_classes(["property"])
                .build()
        };
        let (folder, path) = (property("Folder"), property("Path"));
        let open = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text("Open Containing Folder")
            .valign(gtk::Align::Center)
            .action_name("detail.open-folder")
            .css_classes(["flat"])
            .build();
        folder.add_suffix(&open);
        let file = group("", &folder);
        file.add(&path);
        pane.append(&file);
        let trash = adw::ButtonRow::builder()
            .title("Move to Trash")
            .start_icon_name("user-trash-symbolic")
            .action_name("detail.trash")
            .build();
        // Not through the builder: that would replace the row's own `button` class
        trash.add_css_class("destructive-action");
        pane.append(&group("", &trash));

        // The header. Zooming is left to the image viewer
        let menu = gio::Menu::new();
        menu.append(Some("Open in Image Viewer"), Some("detail.open-image"));
        menu.append(Some("Open Containing Folder"), Some("detail.open-folder"));
        menu.append(Some("Edit Sidecar in Text Editor"), Some("detail.edit-sidecar"));
        let actions = gio::SimpleActionGroup::new();
        for (name, msg) in [
            ("open-image", (|| Msg::Launch(Target::Image)) as fn() -> Msg),
            ("open-folder", || Msg::Launch(Target::Folder)),
            ("edit-sidecar", || Msg::Launch(Target::Sidecar)),
            ("trash", || Msg::Trash),
        ] {
            let s = sender.clone();
            ui::action(&actions, name, move || s.input(msg()));
        }
        page.insert_action_group("detail", Some(&actions));
        let fullscreen = gtk::ToggleButton::builder()
            .icon_name("view-fullscreen-symbolic")
            .tooltip_text("Fullscreen")
            .action_name("win.fullscreen")
            .build();
        let details = gtk::ToggleButton::builder()
            .icon_name("help-about-symbolic")
            .tooltip_text("Details")
            .active(true)
            .build();
        let header = adw::HeaderBar::new();
        header.pack_start(&fullscreen);
        header.pack_end(&ui::menu_button(&menu));
        header.pack_end(&details);
        let banner = adw::Banner::new("Read-only: the sidecar is unreadable or has an unsupported version");

        // In a narrow window the pane lies over the image, so it starts hidden
        let split = adw::OverlaySplitView::builder()
            .sidebar_position(gtk::PackType::End)
            .min_sidebar_width(255.0)
            .content(&image)
            .sidebar(&ui::scrolled(&pane))
            .build();
        details.bind_property("active", &split, "show-sidebar").bidirectional().sync_create().build();
        // RaisedBorder: no shadow over the image
        let view = adw::ToolbarView::builder()
            .content(&split)
            .top_bar_style(adw::ToolbarStyle::RaisedBorder)
            .build();
        view.add_top_bar(&header);
        view.add_top_bar(&banner);
        let bin = ui::bin(&view);
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 590sp").unwrap());
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&split, "show-sidebar", Some(&false.to_value()));
        bin.add_breakpoint(narrow);
        page.set_child(Some(&bin));

        // Otherwise the title field gets the focus with its text selected
        let d = details.downgrade();
        page.connect_shown(move |_| {
            d.upgrade().map(|d| d.grab_focus());
        });
        // Escape leaves fullscreen first, and only then goes back
        let p = page.downgrade();
        ui::shortcut(&page, "Escape", move || {
            match p.upgrade().and_then(|p| p.root()).and_downcast::<gtk::Window>() {
                Some(window) if window.is_fullscreen() => {
                    window.unfullscreen();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });

        let model = Detail {
            lib,
            paths: Vec::new(),
            index: 0,
            meta: Meta::default(),
            editable: false,
            known: Vec::new(),
        };
        let widgets = Widgets {
            picture,
            banner,
            steps,
            details,
            title,
            note,
            tags,
            urls,
            suggestions,
            folder,
            path,
            loading,
        };
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        w: &mut Widgets,
        msg: Msg,
        sender: ComponentSender<Self>,
        page: &adw::NavigationPage,
    ) {
        let window = page.root().and_downcast::<gtk::Window>();
        let output = sender.output_sender().clone();
        let toast = move |text: String| output.emit(Output::Toast(text));
        let opened = {
            let toast = toast.clone();
            move |result: Result<(), glib::Error>| {
                if let Err(e) = result {
                    toast(format!("Could not open: {}", e.message()));
                }
            }
        };
        match msg {
            Msg::Show(paths, index) => {
                (self.paths, self.index) = (paths, index);
                self.load(w, &sender, page);
            }
            Msg::Step(by) => {
                if let Some(index) = self.index.checked_add_signed(by).filter(|i| *i < self.paths.len()) {
                    self.index = index;
                    self.load(w, &sender, page);
                }
            }
            Msg::ToggleDetails => w.details.set_active(!w.details.is_active()),
            Msg::Title(title) => {
                self.meta.title = title;
                self.save(&sender);
            }
            Msg::Note(note) => {
                self.meta.note = note;
                self.save(&sender);
            }
            Msg::Add(field, value) => {
                w.list(field).entry.set_text("");
                if library::push_unique(self.values(field), &value) {
                    self.changed(w, field, &sender);
                }
            }
            Msg::Row(field, act, index) => {
                let Some(value) = self.values(field).get(index).cloned() else { return };
                match (act, field) {
                    (Act::Remove, _) => {
                        self.values(field).remove(index);
                        self.changed(w, field, &sender);
                    }
                    (Act::Copy, _) => {
                        page.clipboard().set_text(&value);
                        toast("Copied".into());
                    }
                    (Act::Open, Field::Tags) => drop(sender.output(Output::ShowTag(value))),
                    (Act::Open, Field::Urls) => {
                        gtk::UriLauncher::new(&value).launch(window.as_ref(), gio::Cancellable::NONE, opened);
                    }
                }
            }
            Msg::Typed(text) => {
                let found = library::suggest(&self.known, &self.meta.tags, &text);
                fill(&w.suggestions, &found, |_, tag| suggestion_row(tag, &sender));
            }
            Msg::Launch(target) => {
                let path = self.path().to_owned();
                let mut lib = self.lib.borrow_mut();
                let file = lib.file(&path);
                let launch = |file: &Path| gtk::FileLauncher::new(Some(&gio::File::for_path(file)));
                match target {
                    Target::Image => launch(&file).launch(window.as_ref(), gio::Cancellable::NONE, opened),
                    Target::Folder => {
                        launch(&file).open_containing_folder(window.as_ref(), gio::Cancellable::NONE, opened)
                    }
                    Target::Sidecar => {
                        if let Err(e) = lib.create(&path, &ui::now()) {
                            toast(format!("Could not create the sidecar: {e}"));
                        }
                        launch(&lib.sidecar(&path)).launch(window.as_ref(), gio::Cancellable::NONE, opened);
                    }
                }
            }
            Msg::Trash => {
                let name = self.path().file_name().unwrap_or_default().to_string_lossy();
                let dialog = adw::AlertDialog::new(
                    Some("Move to Trash?"),
                    Some(&format!("“{name}” and its sidecar will be moved to the trash.")),
                );
                dialog.add_responses(&[("cancel", "_Cancel"), ("trash", "_Move to Trash")]);
                dialog.set_response_appearance("trash", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");
                dialog.connect_response(Some("trash"), move |_, _| sender.input(Msg::Trashed));
                dialog.present(Some(page));
            }
            // The image and its sidecar. To the trash, not deleted, so that it can be undone in
            // Files
            Msg::Trashed => {
                let lib = self.lib.borrow();
                // Not the folder itself, which is the file of the empty ID
                if lib.get(self.path()).is_none() {
                    return;
                }
                let (file, sidecar) = (lib.file(self.path()), lib.sidecar(self.path()));
                drop(lib);
                let trash = |p: &Path| match p.exists() {
                    true => gio::File::for_path(p).trash(gio::Cancellable::NONE),
                    false => Ok(()),
                };
                match trash(&file).and_then(|_| trash(&sidecar)) {
                    Ok(()) => toast("Moved to Trash".into()),
                    Err(e) => toast(format!("Could not move to the trash: {}", e.message())),
                }
                let _ = sender.output(Output::Trashed);
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        w: &mut Widgets,
        Decoded(path, image): Decoded,
        _: ComponentSender<Self>,
        _: &adw::NavigationPage,
    ) {
        if path == self.path() {
            let texture = image.map(|image| {
                let (width, height) = (image.width() as i32, image.height() as i32);
                let bytes = glib::Bytes::from_owned(image.into_raw());
                gdk::MemoryTexture::new(
                    width,
                    height,
                    gdk::MemoryFormat::R8g8b8a8,
                    &bytes,
                    width as usize * 4,
                )
            });
            w.picture.set_paintable(texture.as_ref());
        }
    }
}

// MARK: - Pane

fn group(title: &str, child: &impl IsA<gtk::Widget>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    group.add(child);
    group
}

/// Adds a group to the pane: the rows of a field, under them the row that adds one on Enter, and
/// `below` that
fn list(
    pane: &gtk::Box,
    title: &str,
    add: &str,
    field: Field,
    below: Option<&gtk::ListBox>,
    sender: &ComponentSender<Detail>,
) -> List {
    let list = List { rows: ui::boxed_list(), entry: adw::EntryRow::builder().title(add).build() };
    let s = sender.clone();
    list.entry.connect_entry_activated(move |entry| s.input(Msg::Add(field, entry.text().into())));
    let adder = ui::boxed_list();
    adder.append(&list.entry);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.append(&list.rows);
    column.append(&adder);
    below.iter().for_each(|below| column.append(*below));
    pane.append(&group(title, &column));
    list
}

/// Shows `values` as the rows of `list`; `row` makes one from a position and a value
fn fill(list: &gtk::ListBox, values: &[String], row: impl Fn(usize, &str) -> adw::ActionRow) {
    list.remove_all();
    values.iter().enumerate().for_each(|(i, value)| list.append(&row(i, value)));
    list.set_visible(!values.is_empty());
}

/// A tag or a URL, with a menu
fn value_row(
    field: Field,
    index: usize,
    value: &str,
    editable: bool,
    sender: &ComponentSender<Detail>,
) -> adw::ActionRow {
    // Titles are markup by default, which `&` or `<` would break
    let row =
        adw::ActionRow::builder().title(value).use_markup(false).title_lines(1).activatable(true).build();
    let s = sender.clone();
    row.connect_activated(move |_| s.input(Msg::Row(field, Act::Open, index)));
    let actions = gio::SimpleActionGroup::new();
    for (name, act) in [("open", Act::Open), ("copy", Act::Copy), ("remove", Act::Remove)] {
        let s = sender.clone();
        let action = ui::action(&actions, name, move || s.input(Msg::Row(field, act, index)));
        action.set_enabled(editable || !matches!(act, Act::Remove));
    }
    row.insert_action_group("row", Some(&actions));
    let (menu, last) = (gio::Menu::new(), gio::Menu::new());
    menu.append(Some("Open"), Some("row.open"));
    menu.append(Some("Copy"), Some("row.copy"));
    last.append(Some("Remove"), Some("row.remove"));
    menu.append_section(None, &last);
    if matches!(field, Field::Urls) {
        row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    }
    let more = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .menu_model(&menu)
        .build();
    row.add_suffix(&more);
    row
}

/// A tag to add with a click
fn suggestion_row(tag: &str, sender: &ComponentSender<Detail>) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(tag).use_markup(false).activatable(true).build();
    row.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
    let (s, tag) = (sender.clone(), tag.to_owned());
    row.connect_activated(move |_| s.input(Msg::Add(Field::Tags, tag.clone())));
    row
}
