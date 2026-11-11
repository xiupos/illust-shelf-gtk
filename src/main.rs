mod detail;
mod model;

use adw::prelude::*;
use gtk::{gio, glib};
use model::{Config, Library};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const APP_ID: &str = "net.xiupos.IllustViewer";
const ZOOMS: [u32; 3] = [128, 192, 256];
const DEFAULT_ZOOM: u32 = 192;

struct Win {
    nav: adw::NavigationView,
    toasts: adw::ToastOverlay,
    /// Replaced on every reload
    lib: RefCell<Rc<RefCell<Library>>>,
    config: RefCell<Config>,
    /// For zoom
    css: gtk::CssProvider,
}

fn config_path() -> PathBuf {
    glib::user_config_dir().join(APP_ID).join("config.json")
}

fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join(APP_ID).join("thumbs")
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(activate);
    app.set_accels_for_action("win.reload", &["<Control>r"]);
    app.set_accels_for_action("win.choose-folder", &["<Control>o"]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.run()
}

fn activate(app: &adw::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    let nav = adw::NavigationView::new();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&nav));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Illust Viewer")
        .default_width(1100)
        .default_height(760)
        .width_request(360)
        .height_request(300)
        .content(&toasts)
        .build();

    let mut config = Config::load(&config_path());
    if !ZOOMS.contains(&config.zoom) {
        config.zoom = DEFAULT_ZOOM;
    }
    let css = gtk::CssProvider::new();
    let display = WidgetExt::display(&window);
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let win = Rc::new(Win {
        nav,
        toasts,
        lib: RefCell::new(Rc::new(RefCell::new(Library::load(None, cache_dir())))),
        config: RefCell::new(config),
        css,
    });
    apply_css(&win);
    let w = Rc::downgrade(&win);
    adw::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(w) = w.upgrade() {
            apply_css(&w);
        }
    });
    add_actions(app, &window, &win);
    window.present();
    reload(&win);
}

/// Adds `win.<name>`, calling `f` with its parameter
fn action(
    window: &adw::ApplicationWindow,
    name: &str,
    ty: Option<&glib::VariantTy>,
    f: impl Fn(Option<&glib::Variant>) + 'static,
) {
    let action = gio::SimpleAction::new(name, ty);
    action.connect_activate(move |_, param| f(param));
    window.add_action(&action);
}

fn add_actions(app: &adw::Application, window: &adw::ApplicationWindow, win: &Rc<Win>) {
    let string = Some(glib::VariantTy::STRING);
    let w = win.clone();
    action(window, "reload", None, move |_| reload(&w));
    let w = win.clone();
    action(window, "choose-folder", None, move |_| choose_folder(&w));
    let w = win.clone();
    action(window, "add-images", None, move |_| choose_images(&w));
    // Ctrl+V on the top page adds images from the clipboard. On the window, in the bubble phase, so
    // it also works with nothing focused, and a focused text field pastes text instead
    let w = win.clone();
    let ctrl_v = gtk::CallbackAction::new(move |_, _| {
        if w.nav.navigation_stack().n_items() != 1 {
            return glib::Propagation::Proceed;
        }
        paste(&w);
        glib::Propagation::Stop
    });
    let shortcuts = gtk::ShortcutController::new();
    shortcuts
        .add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("<Control>v"), Some(ctrl_v)));
    window.add_controller(shortcuts);
    // Images dropped anywhere on the window are added too
    let drop = gtk::DropTarget::new(gtk::gdk::FileList::static_type(), gtk::gdk::DragAction::COPY);
    let w = win.clone();
    drop.connect_drop(move |_, value, _, _| {
        let Ok(files) = value.get::<gtk::gdk::FileList>() else { return false };
        add_images(&w, files.files().iter().filter_map(|f| f.path()).collect());
        true
    });
    window.add_controller(drop);
    // (path, tag, query): the tag ("" for all) and query of the gallery it was opened from, so
    // the detail page can step through the same list. show-sibling replaces the detail page
    let item = Some(glib::VariantTy::new("(sss)").unwrap());
    for (name, replace) in [("show-item", false), ("show-sibling", true)] {
        let w = win.clone();
        action(window, name, item, move |param| {
            let Some((path, tag, query)) = param.and_then(|p| p.get::<(String, String, String)>()) else {
                return;
            };
            let lib = w.lib.borrow().clone();
            let page = detail::page(&lib, Path::new(&path), &tag, &query);
            if !replace {
                return w.nav.push(&page);
            }
            save_visible(&w);
            let stack = w.nav.navigation_stack();
            let mut pages: Vec<adw::NavigationPage> = stack.iter().filter_map(|p| p.ok()).collect::<Vec<_>>();
            pages.pop();
            pages.push(page);
            w.nav.replace(&pages);
        });
    }
    let w = win.clone();
    action(window, "show-scope", string, move |param| {
        if let Some(tag) = param.and_then(|p| p.str()) {
            w.nav.push(&scope_page(&w, tag));
        }
    });
    let w = win.clone();
    action(window, "show-namespace", string, move |param| {
        if let Some(space) = param.and_then(|p| p.str()) {
            w.nav.push(&namespace_page(&w, space));
        }
    });
    let w = win.clone();
    action(window, "toast", string, move |param| {
        if let Some(text) = param.and_then(|p| p.str()) {
            w.toasts.add_toast(adw::Toast::new(text));
        }
    });

    let zoom = win.config.borrow().zoom.to_variant();
    let zoom = gio::SimpleAction::new_stateful("zoom", Some(glib::VariantTy::UINT32), &zoom);
    let w = win.clone();
    zoom.connect_activate(move |action, param| {
        if let Some(z) = param.and_then(|p| p.get::<u32>()).filter(|z| ZOOMS.contains(z)) {
            action.set_state(&z.to_variant());
            w.config.borrow_mut().zoom = z;
            apply_css(&w);
            save_config(&w);
        }
    });
    window.add_action(&zoom);

    // Ctrl+Plus / Ctrl+Minus step from the current size and call win.zoom
    let shortcuts = gtk::ShortcutController::new();
    for (trigger, step) in [
        ("<Control>plus|<Control>equal|<Control>KP_Add", 1),
        ("<Control>minus|<Control>KP_Subtract", -1),
        ("<Control>0|<Control>KP_0", 0),
    ] {
        let w = win.clone();
        let action = gtk::CallbackAction::new(move |widget, _| {
            let current = ZOOMS.iter().position(|&z| z == w.config.borrow().zoom).unwrap_or(1);
            let next = match step {
                0 => DEFAULT_ZOOM,
                _ => ZOOMS[current.saturating_add_signed(step).min(ZOOMS.len() - 1)],
            };
            let _ = widget.activate_action("win.zoom", Some(&next.to_variant()));
            glib::Propagation::Stop
        });
        let trigger = gtk::ShortcutTrigger::parse_string(trigger);
        shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(action)));
    }
    window.add_controller(shortcuts);

    let about = gio::SimpleAction::new("about", None);
    let w = win.clone();
    about.connect_activate(move |_, _| {
        let dialog = adw::AboutDialog::builder()
            .application_name("Illust Viewer")
            .application_icon("image-x-generic")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Browse and organize illustrations with sidecar metadata")
            .build();
        dialog.present(Some(&w.nav));
    });
    app.add_action(&about);

    // Closing the window (instead of app.quit) runs close-request, which saves the detail page
    let quit = gio::SimpleAction::new("quit", None);
    let weak = window.downgrade();
    quit.connect_activate(move |_, _| weak.upgrade().iter().for_each(|w| w.close()));
    app.add_action(&quit);
    let w = win.clone();
    window.connect_close_request(move |_| {
        save_visible(&w);
        glib::Propagation::Proceed
    });
}

/// Saves the detail page if one is shown. Pages below it were saved when they were hidden
fn save_visible(win: &Win) {
    if let Some(page) = win.nav.visible_page() {
        let _ = page.activate_action(detail::SAVE, None);
    }
}

/// The app's CSS: the gallery at the current zoom, and the detail page's image controls and bottom
/// pane from GNOME's Image Viewer. Its OSD color depends on libadwaita's dark style, which
/// `prefers-color-scheme` does not follow when the app or a debug variable overrides it
fn apply_css(win: &Win) {
    let n = win.config.borrow().zoom;
    let osd = if adw::StyleManager::default().is_dark() {
        "rgba(0, 0, 0, 0.65)"
    } else {
        "rgba(240, 240, 240, 0.75)"
    };
    win.css.load_from_string(&format!(
        "gridview.gallery {{ padding: 6px; }}
         gridview.gallery > child {{ padding: 3px; }}
         gridview.gallery picture {{ min-width: {n}px; min-height: {n}px; border-radius: 6px; }}
         .osd-box {{ border-radius: 12px; background-color: {osd}; padding: 6px; margin: 18px; }}
         .tag-tile .shade {{ border-radius: 6px; background: linear-gradient(to bottom, transparent 40%, rgba(0, 0, 0, 0.65)); }}
         .tag-tile label {{ color: white; text-shadow: 0 1px 3px rgba(0, 0, 0, 0.6); }}
         .tag-tile .tag {{ font-weight: 800; font-size: 1.1em; }}
         .tag-tile .count {{ font-size: 0.85em; opacity: 0.85; }}
         .bottom-details {{ box-shadow: 0 -1px var(--headerbar-darker-shade-color); }}"
    ));
}

fn save_config(win: &Win) {
    if let Err(e) = win.config.borrow().save(&config_path()) {
        win.toasts.add_toast(adw::Toast::new(&format!("Could not save settings: {e}")));
    }
}

fn choose_folder(win: &Rc<Win>) {
    let dialog = gtk::FileDialog::builder().title("Choose Folder").modal(true).build();
    let parent = win.nav.root().and_downcast::<gtk::Window>();
    let win = win.clone();
    dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
        if let Some(path) = result.ok().and_then(|f| f.path()) {
            win.config.borrow_mut().folder = Some(path);
            save_config(&win);
            reload(&win);
        }
    });
}

/// Called on startup, Ctrl+R, and folder changes. Replaces the whole screen
fn choose_images(win: &Rc<Win>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    for ext in model::EXTENSIONS {
        filter.add_suffix(ext);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder().title("Add Images").modal(true).filters(&filters).build();
    let parent = win.nav.root().and_downcast::<gtk::Window>();
    let win = win.clone();
    dialog.open_multiple(parent.as_ref(), gio::Cancellable::NONE, move |result| {
        if let Ok(files) = result {
            let files = files.iter::<gio::File>().filter_map(|f| f.ok()?.path()).collect();
            add_images(&win, files);
        }
    });
}

/// Copies the images into the folder, each with an empty sidecar (`v` and `added`), then reloads
/// An image to add: a file to copy, or PNG data pasted from the clipboard
enum Source {
    File(PathBuf),
    Png(Vec<u8>),
}

fn add_images(win: &Rc<Win>, files: Vec<PathBuf>) {
    add_sources(win, files.into_iter().map(Source::File).collect());
}

/// Copies or saves the images into the folder, each with an empty sidecar (`v` and `added`), then
/// reloads
fn add_sources(win: &Rc<Win>, sources: Vec<Source>) {
    let Some(folder) = win.config.borrow().folder.clone() else {
        win.toasts.add_toast(adw::Toast::new("Choose a folder first"));
        return;
    };
    let (win, now, total) = (win.clone(), detail::now(), sources.len());
    glib::spawn_future_local(async move {
        // The first failure, as "name: error"
        let result = gio::spawn_blocking(move || {
            let errors: Vec<String> = sources
                .iter()
                .filter_map(|source| {
                    let (name, result) = match source {
                        Source::File(f) => (
                            f.file_name().unwrap_or_default().to_string_lossy(),
                            model::import(&folder, f, &now),
                        ),
                        Source::Png(png) => ("Pasted image".into(), model::import_png(&folder, png, &now)),
                    };
                    Some(format!("{name}: {}", result.err()?))
                })
                .collect();
            (errors.len(), errors.into_iter().next())
        })
        .await;
        let (failed, first) = result.unwrap_or((total, None));
        reload(&win);
        let added = total - failed;
        let message = match first {
            None => format!("Added {added} image{}", if added == 1 { "" } else { "s" }),
            Some(e) => format!("Added {added}, {failed} failed. {e}"),
        };
        win.toasts.add_toast(adw::Toast::new(&message));
    });
}

/// Adds copied image files as they are, or else copied image data as PNG
fn paste(win: &Rc<Win>) {
    let clipboard = win.nav.clipboard();
    let win = win.clone();
    glib::spawn_future_local(async move {
        // Another app's clipboard lists only MIME types; add the types GTK can read them as
        let formats = clipboard.formats().union_deserialize_types();
        let sources = if formats.contains_type(gtk::gdk::FileList::static_type()) {
            let value =
                clipboard.read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT).await;
            let files = value.ok().and_then(|v| v.get::<gtk::gdk::FileList>().ok());
            files.map(|f| f.files().iter().filter_map(|f| f.path()).map(Source::File).collect())
        } else if formats.contains_type(gtk::gdk::Texture::static_type()) {
            let texture = clipboard.read_texture_future().await.ok().flatten();
            texture.map(|t| vec![Source::Png(t.save_to_png_bytes().to_vec())])
        } else {
            None
        };
        match sources {
            Some(sources) if !sources.is_empty() => add_sources(&win, sources),
            _ => win.toasts.add_toast(adw::Toast::new("No image to paste")),
        }
    });
}

fn reload(win: &Rc<Win>) {
    save_visible(win);
    let loading = loading_page();
    win.nav.replace(std::slice::from_ref(&loading));
    let folder = win.config.borrow().folder.clone();
    let win = win.clone();
    glib::spawn_future_local(async move {
        let lib = gio::spawn_blocking(move || Library::load(folder.as_deref(), cache_dir()))
            .await
            .unwrap_or_else(|_| Library::load(None, cache_dir()));
        // If a newer reload has started, let it win
        if win.nav.visible_page().as_ref() != Some(&loading) {
            return;
        }
        *win.lib.borrow_mut() = Rc::new(RefCell::new(lib));
        win.nav.replace(&[root_page(&win)]);
    });
}

/// The primary menu (HIG): view options, then app items, ending with About
fn main_menu() -> gio::Menu {
    let zoom = gio::Menu::new();
    for (label, z) in [("Small", 128u32), ("Medium", 192), ("Large", 256)] {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some("win.zoom"), Some(&z.to_variant()));
        zoom.append_item(&item);
    }
    let folder = gio::Menu::new();
    folder.append(Some("Choose Folder…"), Some("win.choose-folder"));
    folder.append(Some("Reload"), Some("win.reload"));
    let app = gio::Menu::new();
    app.append(Some("About Illust Viewer"), Some("app.about"));
    let menu = gio::Menu::new();
    menu.append_section(Some("Thumbnail Size"), &zoom);
    menu.append_section(None, &folder);
    menu.append_section(None, &app);
    menu
}

fn menu_button() -> gtk::MenuButton {
    detail::menu_button(&main_menu())
}

fn status_page(status: &adw::StatusPage, with_menu: bool) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    if with_menu {
        header.pack_end(&menu_button());
    }
    let view = adw::ToolbarView::builder().content(status).build();
    view.add_top_bar(&header);
    adw::NavigationPage::new(&view, "Illust Viewer")
}

fn loading_page() -> adw::NavigationPage {
    let status = adw::StatusPage::builder().title("Creating Thumbnails…").build();
    status.set_paintable(Some(&adw::SpinnerPaintable::new(Some(&status))));
    status_page(&status, false)
}

fn empty_page(win: &Rc<Win>) -> adw::NavigationPage {
    let chosen = win.config.borrow().folder.is_some();
    let button = gtk::Button::builder()
        .label("Choose Folder")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .action_name("win.choose-folder")
        .build();
    let status = adw::StatusPage::builder()
        .icon_name("folder-pictures-symbolic")
        .title(if chosen { "No Illustrations" } else { "No Folder" })
        .description("Choose a folder that contains illustrations. Subfolders and symlinks are included.")
        .child(&button)
        .build();
    status_page(&status, true)
}

fn root_page(win: &Rc<Win>) -> adw::NavigationPage {
    let lib = win.lib.borrow().clone();
    if lib.borrow().is_empty() {
        return empty_page(win);
    }
    let entry = gtk::SearchEntry::builder().placeholder_text("Search").build();
    let stack = adw::ViewStack::new();
    let gallery = gallery(&lib, None, &entry);
    stack.add_titled_with_icon(&gallery, Some("gallery"), "Illustrations", "image-x-generic-symbolic");
    let tags = tag_sections(&lib, &entry);
    stack.add_titled_with_icon(&tags, Some("tags"), "Tags", "user-bookmarks-symbolic");

    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Images")
        .action_name("win.add-images")
        .build();
    header.pack_start(&add);
    header.pack_end(&menu_button());
    let page = searchable_page("Illust Viewer", &header, &entry, &stack);

    // On narrow windows the switcher moves from the header to a bottom bar (libadwaita ViewSwitcher)
    let bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    let bin = page.child().and_downcast::<adw::BreakpointBin>().unwrap();
    bin.child().and_downcast::<adw::ToolbarView>().unwrap().add_bottom_bar(&bar);
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 550sp").unwrap());
    narrow.add_setter(&bar, "reveal", Some(&true.to_value()));
    narrow.add_setter(&switcher, "visible", Some(&false.to_value()));
    bin.add_breakpoint(narrow);
    page
}

/// All tags of one namespace ("" for tags without one). Has its own header and search bar
fn namespace_page(win: &Rc<Win>, space: &str) -> adw::NavigationPage {
    let lib = win.lib.borrow().clone();
    let entry = gtk::SearchEntry::builder().placeholder_text("Search").build();
    let grid = tag_grid(&lib, &entry, Some(space.to_owned()));
    searchable_page(&space_title(space), &adw::HeaderBar::new(), &entry, &grid)
}

/// Gallery of one tag. Has its own header and search bar
fn scope_page(win: &Rc<Win>, tag: &str) -> adw::NavigationPage {
    let lib = win.lib.borrow().clone();
    let entry = gtk::SearchEntry::builder().placeholder_text("Search").build();
    let gallery = gallery(&lib, Some(tag.to_owned()), &entry);
    searchable_page(tag, &adw::HeaderBar::new(), &entry, &gallery)
}

/// Page with a header, a search bar below it (HIG: toggled by a header button or Ctrl+F, and
/// typing starts a search while the page is shown), and the content, in a BreakpointBin
fn searchable_page(
    title: &str,
    header: &adw::HeaderBar,
    entry: &gtk::SearchEntry,
    content: &impl IsA<gtk::Widget>,
) -> adw::NavigationPage {
    let clamp = adw::Clamp::builder().maximum_size(400).child(entry).build();
    let bar = gtk::SearchBar::builder().child(&clamp).build();
    bar.connect_entry(entry);
    let toggle =
        gtk::ToggleButton::builder().icon_name("system-search-symbolic").tooltip_text("Search").build();
    toggle.bind_property("active", &bar, "search-mode-enabled").bidirectional().build();
    header.pack_end(&toggle);
    bar.connect_map(|bar| bar.set_key_capture_widget(bar.root().as_ref()));
    bar.connect_unmap(|bar| bar.set_key_capture_widget(gtk::Widget::NONE));

    let view = adw::ToolbarView::builder().content(content).build();
    view.add_top_bar(header);
    view.add_top_bar(&bar);
    let ctrl_f = gtk::CallbackAction::new(move |_, _| {
        bar.set_search_mode(!bar.is_search_mode());
        glib::Propagation::Stop
    });
    let shortcuts = gtk::ShortcutController::new();
    shortcuts
        .add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("<Control>f"), Some(ctrl_f)));
    view.add_controller(shortcuts);
    let bin = adw::BreakpointBin::builder().width_request(360).height_request(300).child(&view).build();
    adw::NavigationPage::new(&bin, title)
}

/// Built once with the library at hand; a search rebuilds the whole list
fn gallery(lib: &Rc<RefCell<Library>>, tag: Option<String>, entry: &gtk::SearchEntry) -> gtk::Stack {
    let list = gtk::StringList::new(&lib.borrow().paths(tag.as_deref(), ""));
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, obj| {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .overflow(gtk::Overflow::Hidden)
            .build();
        obj.downcast_ref::<gtk::ListItem>().unwrap().set_child(Some(&picture));
    });
    let l = lib.clone();
    factory.connect_bind(move |_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
        let path = item.item().and_downcast::<gtk::StringObject>().unwrap().string();
        let picture = item.child().and_downcast::<gtk::Picture>().unwrap();
        picture.set_filename(Some(l.borrow().thumb(Path::new(path.as_str()))));
    });
    let grid = tile_grid(&list, &factory);
    // Weak: the entry's handler holds the stack that contains the grid
    let (l, t, e) = (list.clone(), tag.clone().unwrap_or_default(), entry.downgrade());
    grid.connect_activate(move |grid, pos| {
        if let (Some(path), Some(entry)) = (l.string(pos), e.upgrade()) {
            let target = (path.as_str(), t.as_str(), entry.text().as_str()).to_variant();
            let _ = grid.activate_action("win.show-item", Some(&target));
        }
    });
    let stack = no_results(&scrolled(&grid));
    let (lib, st) = (lib.clone(), stack.clone());
    entry.connect_search_changed(move |entry| {
        let lib = lib.borrow();
        list.splice(0, list.n_items(), &lib.paths(tag.as_deref(), &entry.text()));
        show_results(&st, list.n_items() > 0);
    });
    stack
}

/// A grid of square thumbnail tiles at the current zoom
fn tile_grid(list: &gtk::StringList, factory: &gtk::SignalListItemFactory) -> gtk::GridView {
    gtk::GridView::builder()
        .model(&gtk::NoSelection::new(Some(list.clone())))
        .factory(factory)
        .single_click_activate(true)
        .max_columns(64)
        // Columns come from the minimum width (the zoom), then stretch to fill the row. A natural
        // vertical policy makes each row as tall as a square thumbnail at that width, so tiles
        // stay square without gaps
        .vscroll_policy(gtk::ScrollablePolicy::Natural)
        .css_classes(["gallery"])
        .build()
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(child).build()
}

/// Tag → (count, cover image), from the last time the view was shown
type TagInfo = Rc<RefCell<HashMap<String, (usize, PathBuf)>>>;

/// Fills `info` and returns the tags, most used first
fn load_tags(lib: &Rc<RefCell<Library>>, info: &TagInfo) -> Vec<String> {
    let mut info = info.borrow_mut();
    info.clear();
    let facets = lib.borrow().facets();
    facets
        .into_iter()
        .map(|(tag, count, cover)| {
            info.insert(tag.clone(), (count, cover));
            tag
        })
        .collect()
}

/// Tag tiles like pixiv's tag search: the tag's cover image, a dark gradient, "#tag", and its count.
/// `short` drops the namespace from the label, where a heading or title already names it
fn tag_factory(lib: &Rc<RefCell<Library>>, info: &TagInfo, short: bool) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, obj| {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .overflow(gtk::Overflow::Hidden)
            .build();
        let label = |class: &str| {
            gtk::Label::builder()
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(2)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .justify(gtk::Justification::Center)
                .css_classes([class])
                .build()
        };
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::End)
            .margin_start(8)
            .margin_end(8)
            .margin_bottom(10)
            .build();
        text.append(&label("tag"));
        text.append(&label("count"));
        let tile = gtk::Overlay::builder().child(&picture).css_classes(["tag-tile"]).build();
        tile.add_overlay(&gtk::Box::builder().css_classes(["shade"]).build());
        tile.add_overlay(&text);
        obj.downcast_ref::<gtk::ListItem>().unwrap().set_child(Some(&tile));
    });
    let (l, info) = (lib.clone(), info.clone());
    factory.connect_bind(move |_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
        let tag = item.item().and_downcast::<gtk::StringObject>().unwrap().string();
        let tile = item.child().and_downcast::<gtk::Overlay>().unwrap();
        let Some((count, cover)) = info.borrow().get(tag.as_str()).cloned() else { return };
        tile.child().and_downcast::<gtk::Picture>().unwrap().set_filename(Some(l.borrow().thumb(&cover)));
        let text = tile.last_child().unwrap();
        let tag_label = text.first_child().and_downcast::<gtk::Label>().unwrap();
        let name =
            if short { tag.split_once(':').map_or(tag.as_str(), |(_, name)| name) } else { tag.as_str() };
        tag_label.set_label(&format!("#{name}"));
        tag_label.next_sibling().and_downcast::<gtk::Label>().unwrap().set_label(&count.to_string());
        tile.set_tooltip_text(Some(&tag));
    });
    factory
}

/// A grid of tag tiles; activating one opens its gallery
fn tag_tiles(list: &gtk::StringList, factory: &gtk::SignalListItemFactory) -> gtk::GridView {
    let grid = tile_grid(list, factory);
    let l = list.clone();
    grid.connect_activate(move |grid, pos| {
        if let Some(tag) = l.string(pos) {
            let _ = grid.activate_action("win.show-scope", Some(&tag.to_variant()));
        }
    });
    grid
}

/// Every tag, or those of one namespace ("" for tags without one), as tiles. Searching filters
/// them by name, so `creator:` shows the creators. Rebuilt every time it is shown
fn tag_grid(lib: &Rc<RefCell<Library>>, entry: &gtk::SearchEntry, space: Option<String>) -> gtk::Stack {
    let info: TagInfo = Rc::default();
    let tags: Rc<RefCell<Vec<String>>> = Rc::default();
    let list = gtk::StringList::new(&[]);
    let stack = no_results(&scrolled(&tag_tiles(&list, &tag_factory(lib, &info, space.is_some()))));

    // Weak: the stack's map handler holds `update`
    let (t, st) = (tags.clone(), stack.downgrade());
    let update = Rc::new(move |entry: &gtk::SearchEntry| {
        let query = entry.text().trim().to_lowercase();
        let tags = t.borrow();
        let shown: Vec<&str> =
            tags.iter().map(String::as_str).filter(|t| t.to_lowercase().contains(&query)).collect();
        list.splice(0, list.n_items(), &shown);
        if let Some(st) = st.upgrade() {
            show_results(&st, !shown.is_empty());
        }
    });
    let u = update.clone();
    entry.connect_search_changed(move |entry| u(entry));
    let (lib, weak) = (lib.clone(), entry.downgrade());
    stack.connect_map(move |_| {
        let mut all = load_tags(&lib, &info);
        if let Some(space) = &space {
            all.retain(|t| model::namespace(t) == space);
        }
        *tags.borrow_mut() = all;
        if let Some(entry) = weak.upgrade() {
            update(&entry);
        }
    });
    stack
}

/// "Creator" for `creator:`; "Other Tags" for tags without a namespace
fn space_title(space: &str) -> String {
    let mut chars = space.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Other Tags".into(),
    }
}

/// The Tags tab: a section per namespace (most used first, tags without one last), each a header
/// that opens all its tags and one row of its most used tags, as many as fit. While searching, a
/// flat grid of every matching tag instead. Rebuilt every time it is shown
fn tag_sections(lib: &Rc<RefCell<Library>>, entry: &gtk::SearchEntry) -> gtk::Stack {
    let info: TagInfo = Rc::default();
    let factory = tag_factory(lib, &info, true);
    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_bottom(12).build();
    let sections_view = scrolled(&column);
    // Each section's grid, its shown tags, and all its tags
    type Sections = Rc<RefCell<Vec<(gtk::GridView, gtk::StringList, Vec<String>)>>>;
    let sections: Sections = Rc::default();
    let zoom = Rc::new(std::cell::Cell::new(DEFAULT_ZOOM));
    // One row per section, by the grid's own column rule: tiles at least the zoom wide plus 6 px
    // padding, inside 12 px of grid padding. Weak: the adjustment's handler holds this
    let (s, z, h) = (sections.clone(), zoom.clone(), sections_view.hadjustment().downgrade());
    let fit = Rc::new(move || {
        let Some(h) = h.upgrade() else { return };
        let n = ((h.page_size() as u32).saturating_sub(12) / (z.get() + 6)).max(1);
        for (grid, list, tags) in s.borrow().iter() {
            // Only a maximum: a minimum would fix the grid's width, and the view could not shrink
            grid.set_max_columns(n);
            let shown: Vec<&str> = tags.iter().take(n as usize).map(String::as_str).collect();
            list.splice(0, list.n_items(), &shown);
        }
    });
    let f = fit.clone();
    sections_view.hadjustment().connect_page_size_notify(move |_| f());

    let stack = gtk::Stack::new();
    stack.add_named(&sections_view, Some("sections"));
    stack.add_named(&tag_grid(lib, entry, None), Some("search"));
    let st = stack.clone();
    let show = move |entry: &gtk::SearchEntry| {
        st.set_visible_child_name(if entry.text().trim().is_empty() { "sections" } else { "search" });
    };
    entry.connect_search_changed(show.clone());

    // The zoom changes the fit; follow the window's zoom action while shown
    let handler: Rc<RefCell<Option<(gio::SimpleAction, glib::SignalHandlerId)>>> = Rc::default();
    let (lib, weak, h) = (lib.clone(), entry.downgrade(), handler.clone());
    sections_view.connect_map(move |view| {
        while let Some(child) = column.first_child() {
            column.remove(&child);
        }
        let all = load_tags(&lib, &info);
        let mut spaces = lib.borrow().namespaces();
        // Tags without a namespace go last
        spaces.sort_by_key(|(space, _)| space.is_empty());
        let mut s = sections.borrow_mut();
        s.clear();
        for (space, _) in spaces {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content
                .append(&gtk::Label::builder().label(space_title(&space)).css_classes(["title-4"]).build());
            content.append(&gtk::Image::from_icon_name("go-next-symbolic"));
            let header = gtk::Button::builder()
                .child(&content)
                .halign(gtk::Align::Start)
                .margin_start(6)
                .margin_top(12)
                .css_classes(["flat"])
                .action_name("win.show-namespace")
                .action_target(&space.to_variant())
                .build();
            let list = gtk::StringList::new(&[]);
            let grid = tag_tiles(&list, &factory);
            column.append(&header);
            column.append(&grid);
            let tags = all.iter().filter(|t| model::namespace(t) == space).cloned().collect();
            s.push((grid, list, tags));
        }
        drop(s);
        let action = view
            .root()
            .and_downcast::<gtk::ApplicationWindow>()
            .and_then(|w| w.lookup_action("zoom"))
            .and_downcast::<gio::SimpleAction>();
        if let Some(action) = action {
            zoom.set(action.state().and_then(|v| v.get::<u32>()).unwrap_or(DEFAULT_ZOOM));
            let (z, f) = (zoom.clone(), fit.clone());
            let id = action.connect_state_notify(move |action| {
                z.set(action.state().and_then(|v| v.get::<u32>()).unwrap_or(DEFAULT_ZOOM));
                f();
            });
            h.replace(Some((action, id)));
        }
        fit();
        if let Some(entry) = weak.upgrade() {
            show(&entry);
        }
    });
    sections_view.connect_unmap(move |_| {
        if let Some((action, id)) = handler.take() {
            action.disconnect(id);
        }
    });
    stack
}

/// Shows `content`, or a "No Results" page while a search finds nothing (HIG)
fn no_results(content: &impl IsA<gtk::Widget>) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.add_named(content, Some("content"));
    let status = adw::StatusPage::builder().icon_name("edit-find-symbolic").title("No Results").build();
    stack.add_named(&status, Some("empty"));
    stack
}

fn show_results(stack: &gtk::Stack, any: bool) {
    stack.set_visible_child_name(if any { "content" } else { "empty" });
}
