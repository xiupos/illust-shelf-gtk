//! The window: the pages and the moves between them, settings, zoom, loading, and adding images.
//!
//! The pages are fixed: the top page (Illustrations and Tags), one tag's gallery, and one image's
//! details. Each exists once and is shown with new content.

mod browse;
mod detail;
mod library;
mod ui;

use adw::prelude::*;
use browse::{Gallery, GalleryMsg, Open, Tags, TagsMsg, TagsOutput};
use detail::Detail;
use gtk::{gdk, gio, glib};
use library::Library;
use relm4::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::{fs, io};
use ui::Lib;

const APP_ID: &str = "net.xiupos.IllustShelf.Gtk";
/// Thumbnail sizes in px: Small, Medium, Large
const ZOOMS: [u32; 3] = [128, 192, 256];
const DEFAULT_ZOOM: u32 = 192;

// MARK: - App

struct App {
    config: Config,
    lib: Lib,
    /// Counts the reloads: a load that is not the latest is dropped
    generation: u32,
    gallery: Controller<Gallery>,
    tags: Controller<Tags>,
    tag_gallery: Controller<Gallery>,
    detail: Controller<Detail>,
}

struct Widgets {
    nav: adw::NavigationView,
    toasts: adw::ToastOverlay,
    top: adw::NavigationPage,
    top_search: ui::SearchView,
    tag: adw::NavigationPage,
    tag_search: ui::SearchView,
    loading: adw::NavigationPage,
    empty: adw::NavigationPage,
    zoom: gio::SimpleAction,
}

#[derive(Debug)]
enum Msg {
    Reload,
    ChooseFolder,
    Folder(PathBuf),
    ChooseImages,
    Paste,
    Add(Vec<Source>),
    Zoom(u32),
    /// A size up or down, or back to the default
    ZoomStep(Option<isize>),
    ShowTag(String),
    ShowItem(Open),
    /// Searches the top page
    Search(String),
    TagsChanged,
    Toast(String),
}

#[derive(Debug)]
enum Cmd {
    Loaded(u32, Library),
    /// The number of images, of those that failed, and the first failure
    Added(usize, usize, Option<String>),
}

// MARK: - Adding images

/// An image to add: a file to copy, or PNG data from the clipboard
#[derive(Debug)]
enum Source {
    File(PathBuf),
    Png(Vec<u8>),
}

fn sources(files: &gdk::FileList) -> Vec<Source> {
    files.files().iter().filter_map(|f| f.path()).map(Source::File).collect()
}

// MARK: - Settings

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub folder: Option<PathBuf>,
    /// The thumbnail size in px
    pub zoom: u32,
}

impl Config {
    /// The defaults if the file is missing or unreadable
    pub fn load(path: &Path) -> Self {
        fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        library::write_json(path, self)
    }
}

fn config_path() -> PathBuf {
    glib::user_config_dir().join(APP_ID).join("config.json")
}

/// Each folder has its own cache: thumbnails are named by the images' paths in the folder
fn cache_dir(folder: &Path) -> PathBuf {
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, folder.as_os_str().as_encoded_bytes());
    glib::user_cache_dir().join(APP_ID).join("thumbs").join(id.to_string())
}

/// View options, then the folder, then About
fn main_menu() -> gio::Menu {
    let zoom = gio::Menu::new();
    for (label, z) in ["Small", "Medium", "Large"].into_iter().zip(ZOOMS) {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some("win.zoom"), Some(&z.to_variant()));
        zoom.append_item(&item);
    }
    let folder = gio::Menu::new();
    folder.append(Some("Choose Folder…"), Some("win.choose-folder"));
    folder.append(Some("Reload"), Some("win.reload"));
    let app = gio::Menu::new();
    app.append(Some("About Illust Shelf"), Some("app.about"));
    let menu = gio::Menu::new();
    menu.append_section(Some("Thumbnail Size"), &zoom);
    menu.append_section(None, &folder);
    menu.append_section(None, &app);
    menu
}

// MARK: - Pages

fn status_page(status: &adw::StatusPage, header: &adw::HeaderBar) -> adw::NavigationPage {
    let view = adw::ToolbarView::builder().content(status).build();
    view.add_top_bar(header);
    adw::NavigationPage::new(&view, "Illust Shelf")
}

fn loading_page() -> adw::NavigationPage {
    let status = adw::StatusPage::builder().title("Creating Thumbnails…").build();
    status.set_paintable(Some(&adw::SpinnerPaintable::new(Some(&status))));
    status_page(&status, &adw::HeaderBar::new())
}

/// Shown instead of the top page while there are no images
fn empty_page() -> adw::NavigationPage {
    let button = gtk::Button::builder()
        .label("Choose Folder")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .action_name("win.choose-folder")
        .build();
    let status = adw::StatusPage::builder()
        .icon_name("folder-pictures-symbolic")
        .title("No Illustrations")
        .description("Choose a folder that contains illustrations. Subfolders and symlinks are included.")
        .child(&button)
        .build();
    let header = adw::HeaderBar::new();
    header.pack_end(&ui::menu_button(&main_menu()));
    status_page(&status, &header)
}

// MARK: - Window

impl Widgets {
    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }
}

impl App {
    fn save_config(&self, w: &Widgets) {
        if let Err(e) = self.config.save(&config_path()) {
            w.toast(&format!("Could not save settings: {e}"));
        }
    }
}

impl Component for App {
    type Init = ();
    type Input = Msg;
    type Output = ();
    type CommandOutput = Cmd;
    type Root = adw::ApplicationWindow;
    type Widgets = Widgets;

    fn init_root() -> adw::ApplicationWindow {
        adw::ApplicationWindow::builder()
            .title("Illust Shelf")
            .default_width(1100)
            .default_height(760)
            .width_request(360)
            .height_request(300)
            .build()
    }

    fn init(_: (), window: adw::ApplicationWindow, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        relm4::set_global_css(include_str!("style.css"));
        // The app's icon, where the app is not installed
        #[cfg(debug_assertions)]
        gtk::IconTheme::for_display(&WidgetExt::display(&window))
            .add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons"));
        let mut config = Config::load(&config_path());
        if !ZOOMS.contains(&config.zoom) {
            config.zoom = DEFAULT_ZOOM;
        }
        window.add_css_class(&format!("zoom-{}", config.zoom));

        let lib = Lib::default();
        let input = sender.input_sender();
        let gallery = Gallery::builder().launch(lib.clone()).forward(input, Msg::ShowItem);
        let tag_gallery = Gallery::builder().launch(lib.clone()).forward(input, Msg::ShowItem);
        let detail = Detail::builder().launch(lib.clone()).forward(input, |output| match output {
            detail::Output::ShowTag(tag) => Msg::ShowTag(tag),
            detail::Output::TagsChanged => Msg::TagsChanged,
            detail::Output::Toast(text) => Msg::Toast(text),
            detail::Output::Trashed => Msg::Reload,
        });
        let tags = Tags::builder().launch(lib.clone()).forward(input, |output| match output {
            TagsOutput::Show(tag) => Msg::ShowTag(tag),
            TagsOutput::Search(query) => Msg::Search(query),
        });

        // The top page. In a narrow window the view switcher moves to a bottom bar
        let stack = adw::ViewStack::new();
        stack.add_titled_with_icon(gallery.widget(), None, "Illustrations", "image-x-generic-symbolic");
        stack.add_titled_with_icon(tags.widget(), None, "Tags", "user-bookmarks-symbolic");
        let switcher =
            adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
        let header = adw::HeaderBar::builder().title_widget(&switcher).build();
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Images")
            .action_name("win.add-images")
            .build();
        header.pack_start(&add);
        header.pack_end(&ui::menu_button(&main_menu()));
        let (g, t) = (gallery.sender().clone(), tags.sender().clone());
        let top_search = ui::search_view(&header, &stack, move |query| {
            g.emit(GalleryMsg::Search(query.clone()));
            t.emit(TagsMsg::Search(query));
        });
        let bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
        top_search.view.add_bottom_bar(&bar);
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 550sp").unwrap());
        narrow.add_setter(&bar, "reveal", Some(&true.to_value()));
        narrow.add_setter(&switcher, "visible", Some(&false.to_value()));
        let bin = ui::bin(&top_search.view);
        bin.add_breakpoint(narrow);
        let top = adw::NavigationPage::new(&bin, "Illust Shelf");

        let g = tag_gallery.sender().clone();
        let tag_search = ui::search_view(&adw::HeaderBar::new(), tag_gallery.widget(), move |query| {
            g.emit(GalleryMsg::Search(query));
        });
        let tag = adw::NavigationPage::new(&ui::bin(&tag_search.view), "");

        let nav = adw::NavigationView::new();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&nav));
        window.set_content(Some(&toasts));

        // Actions for the menus and their keys
        for (name, msg) in [
            ("reload", (|| Msg::Reload) as fn() -> Msg),
            ("choose-folder", || Msg::ChooseFolder),
            ("add-images", || Msg::ChooseImages),
        ] {
            let s = sender.clone();
            ui::action(&window, name, move || s.input(msg()));
        }
        let zoom = config.zoom.to_variant();
        let zoom = gio::SimpleAction::new_stateful("zoom", Some(glib::VariantTy::UINT32), &zoom);
        let s = sender.clone();
        zoom.connect_activate(move |_, zoom| {
            s.input(Msg::Zoom(zoom.and_then(|z| z.get()).unwrap_or(DEFAULT_ZOOM)))
        });
        window.add_action(&zoom);
        // A toggle button with this action shows the window's state
        let fullscreen = gio::SimpleAction::new_stateful("fullscreen", None, &false.to_variant());
        let w = window.downgrade();
        fullscreen.connect_activate(move |_, _| {
            w.upgrade().iter().for_each(|w| w.set_fullscreened(!w.is_fullscreen()));
        });
        window.add_action(&fullscreen);
        window.connect_fullscreened_notify(move |w| fullscreen.set_state(&w.is_fullscreen().to_variant()));

        let app = relm4::main_application();
        let w = window.downgrade();
        ui::action(&app, "about", move || {
            let dialog = adw::AboutDialog::builder()
                .application_name("Illust Shelf")
                .application_icon(APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .comments("Browse and organize illustrations with tags")
                .website("https://github.com/xiupos/illust-viewer-gtk")
                .issue_url("https://github.com/xiupos/illust-viewer-gtk/issues")
                .license_type(gtk::License::MitX11)
                .build();
            dialog.present(w.upgrade().as_ref());
        });
        let a = app.downgrade();
        ui::action(&app, "quit", move || a.upgrade().iter().for_each(|a| a.quit()));
        for (action, keys) in [
            ("win.reload", "<Control>r"),
            ("win.choose-folder", "<Control>o"),
            ("win.fullscreen", "F11"),
            ("app.quit", "<Control>q"),
        ] {
            app.set_accels_for_action(action, &[keys]);
        }
        for (trigger, step) in [
            ("<Control>plus|<Control>equal|<Control>KP_Add", Some(1)),
            ("<Control>minus|<Control>KP_Subtract", Some(-1)),
            ("<Control>0|<Control>KP_0", None),
        ] {
            let s = sender.clone();
            ui::shortcut(&window, trigger, move || {
                s.input(Msg::ZoomStep(step));
                glib::Propagation::Stop
            });
        }

        // On the window, so that they work wherever the focus is. A focused text field still gets
        // the arrow keys first
        for (key, msg) in [
            ("F9", (|| detail::Msg::ToggleDetails) as fn() -> detail::Msg),
            ("Left", || detail::Msg::Step(-1)),
            ("Right", || detail::Msg::Step(1)),
        ] {
            let (n, page, d) = (nav.downgrade(), detail.widget().downgrade(), detail.sender().clone());
            ui::shortcut(&window, key, move || {
                if n.upgrade().and_then(|n| n.visible_page()) != page.upgrade() {
                    return glib::Propagation::Proceed;
                }
                d.emit(msg());
                glib::Propagation::Stop
            });
        }

        // Adding images: Ctrl+V on the top page, and files dropped on the window. The shortcut is
        // on the window so that it works with nothing focused; a focused text field still pastes
        // text
        let (s, n, t) = (sender.clone(), nav.downgrade(), top.downgrade());
        ui::shortcut(&window, "<Control>v", move || {
            if n.upgrade().and_then(|n| n.visible_page()) != t.upgrade() {
                return glib::Propagation::Proceed;
            }
            s.input(Msg::Paste);
            glib::Propagation::Stop
        });
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let s = sender.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Ok(files) = value.get::<gdk::FileList>() else { return false };
            s.input(Msg::Add(sources(&files)));
            true
        });
        window.add_controller(drop);

        let model = App { config, lib, generation: 0, gallery, tags, tag_gallery, detail };
        let (loading, empty) = (loading_page(), empty_page());
        let widgets = Widgets { nav, toasts, top, top_search, tag, tag_search, loading, empty, zoom };
        sender.input(Msg::Reload);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        w: &mut Widgets,
        msg: Msg,
        sender: ComponentSender<Self>,
        window: &adw::ApplicationWindow,
    ) {
        match msg {
            Msg::Reload => {
                self.generation += 1;
                w.nav.replace(std::slice::from_ref(&w.loading));
                let (generation, folder) = (self.generation, self.config.folder.clone());
                sender.spawn_oneshot_command(move || {
                    let lib = folder.map(|f| Library::load(&f, cache_dir(&f))).unwrap_or_default();
                    Cmd::Loaded(generation, lib)
                });
            }
            Msg::ChooseFolder => {
                let dialog = gtk::FileDialog::builder().title("Choose Folder").modal(true).build();
                dialog.select_folder(Some(window), gio::Cancellable::NONE, move |result| {
                    if let Some(path) = result.ok().and_then(|f| f.path()) {
                        sender.input(Msg::Folder(path));
                    }
                });
            }
            Msg::Folder(path) => {
                self.config.folder = Some(path);
                self.save_config(w);
                sender.input(Msg::Reload);
            }
            Msg::ChooseImages => {
                let filter = gtk::FileFilter::new();
                filter.set_name(Some("Images"));
                for ext in library::EXTENSIONS {
                    filter.add_suffix(ext);
                }
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                let dialog =
                    gtk::FileDialog::builder().title("Add Images").modal(true).filters(&filters).build();
                dialog.open_multiple(Some(window), gio::Cancellable::NONE, move |result| {
                    if let Ok(files) = result {
                        let files = files.iter::<gio::File>().filter_map(|f| f.ok()?.path());
                        sender.input(Msg::Add(files.map(Source::File).collect()));
                    }
                });
            }
            // The clipboard's image files, or else its image as PNG
            Msg::Paste => {
                let clipboard = window.clipboard();
                relm4::spawn_local(async move {
                    // Another app's clipboard lists only MIME types; add the types GTK can read
                    // them as
                    let formats = clipboard.formats().union_deserialize_types();
                    let found = if formats.contains_type(gdk::FileList::static_type()) {
                        let list = gdk::FileList::static_type();
                        let value = clipboard.read_value_future(list, glib::Priority::DEFAULT).await;
                        value.ok().and_then(|v| v.get::<gdk::FileList>().ok()).map(|files| sources(&files))
                    } else if formats.contains_type(gdk::Texture::static_type()) {
                        let texture = clipboard.read_texture_future().await.ok().flatten();
                        texture.map(|t| vec![Source::Png(t.save_to_png_bytes().to_vec())])
                    } else {
                        None
                    };
                    sender.input(match found {
                        Some(found) if !found.is_empty() => Msg::Add(found),
                        _ => Msg::Toast("No image to paste".into()),
                    });
                });
            }
            Msg::Add(sources) => {
                let Some(folder) = self.config.folder.clone() else {
                    return w.toast("Choose a folder first");
                };
                let now = ui::now();
                sender.spawn_oneshot_command(move || {
                    let errors = sources.iter().filter_map(|source| {
                        let (name, result) = match source {
                            Source::File(f) => (
                                f.file_name().unwrap_or_default().to_string_lossy(),
                                library::import(&folder, f, &now),
                            ),
                            Source::Png(png) => {
                                ("Pasted image".into(), library::import_png(&folder, png, &now))
                            }
                        };
                        Some(format!("{name}: {}", result.err()?))
                    });
                    let errors: Vec<String> = errors.collect();
                    Cmd::Added(sources.len(), errors.len(), errors.into_iter().next())
                });
            }
            Msg::Zoom(zoom) => {
                if ZOOMS.contains(&zoom) {
                    window.remove_css_class(&format!("zoom-{}", self.config.zoom));
                    window.add_css_class(&format!("zoom-{zoom}"));
                    w.zoom.set_state(&zoom.to_variant());
                    self.config.zoom = zoom;
                    self.save_config(w);
                }
            }
            Msg::ZoomStep(step) => {
                let current = ZOOMS.iter().position(|&z| z == self.config.zoom).unwrap_or(1);
                sender.input(Msg::Zoom(match step {
                    Some(step) => ZOOMS[current.saturating_add_signed(step).min(ZOOMS.len() - 1)],
                    None => DEFAULT_ZOOM,
                }));
            }
            Msg::ShowTag(tag) => {
                self.tag_gallery.emit(GalleryMsg::Show(Some(tag.clone())));
                w.tag_search.bar.set_search_mode(false);
                w.tag.set_title(&tag);
                // From the detail page, the tag's page takes its place
                if w.nav.visible_page().as_ref() == Some(&w.top) {
                    w.nav.push(&w.tag);
                } else {
                    w.nav.replace(&[w.top.clone(), w.tag.clone()]);
                }
            }
            Msg::ShowItem(open) => {
                self.detail.emit(detail::Msg::Show(open.paths, open.index));
                w.nav.push(self.detail.widget());
            }
            Msg::Search(query) => {
                w.top_search.bar.set_search_mode(true);
                w.top_search.entry.set_text(&query);
                w.top_search.entry.set_position(-1);
            }
            Msg::TagsChanged => self.tags.emit(TagsMsg::Refresh),
            Msg::Toast(text) => w.toast(&text),
        }
    }

    fn update_cmd_with_view(
        &mut self,
        w: &mut Widgets,
        cmd: Cmd,
        sender: ComponentSender<Self>,
        _: &adw::ApplicationWindow,
    ) {
        match cmd {
            Cmd::Loaded(generation, lib) => {
                if generation != self.generation {
                    return;
                }
                let empty = lib.is_empty();
                *self.lib.borrow_mut() = lib;
                self.gallery.emit(GalleryMsg::Show(None));
                self.tags.emit(TagsMsg::Refresh);
                w.top_search.bar.set_search_mode(false);
                w.nav.replace(&[if empty { w.empty.clone() } else { w.top.clone() }]);
            }
            Cmd::Added(total, failed, first) => {
                let added = total - failed;
                w.toast(&match first {
                    None => format!("Added {added} image{}", if added == 1 { "" } else { "s" }),
                    Some(e) => format!("Added {added}, {failed} failed. {e}"),
                });
                sender.input(Msg::Reload);
            }
        }
    }
}

fn main() {
    relm4::RelmApp::new(APP_ID).run::<App>(());
}

// MARK: - Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("illust-shelf-test-{}-config", std::process::id()));
        let path = dir.join("app/config.json");
        assert_eq!(Config::load(&path).zoom, 0);
        Config { folder: Some("/a".into()), zoom: 256 }.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!((loaded.folder, loaded.zoom), (Some("/a".into()), 256));
    }
}
