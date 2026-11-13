//! The lists of the top page: a gallery of thumbnails (also used for one tag's images), and the
//! Tags tab, with a row of tags per namespace or a grid of the tags that match a search.

use crate::library::{self, Tag};
use crate::ui::{self, Lib};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use relm4::prelude::*;
use relm4::typed_view::grid::{RelmGridItem, TypedGridView};
use std::cell::RefCell;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::rc::Rc;

// MARK: - Gallery

pub struct Gallery {
    lib: Lib,
    tag: Option<String>,
    query: String,
    /// The images shown, in order
    paths: Vec<PathBuf>,
}

pub struct GalleryWidgets {
    /// The grid, or "No Results"
    stack: gtk::Stack,
    grid: TypedGridView<Thumb, gtk::NoSelection>,
}

#[derive(Debug)]
pub enum GalleryMsg {
    /// Shows the images of the tag, or all of them, with no search
    Show(Option<String>),
    Search(String),
    Activated(u32),
    /// Asks, then `Trashed`
    Trash(PathBuf),
    Trashed(PathBuf),
    /// Leaves out an image that is gone, keeping the scroll position
    Removed(PathBuf),
}

#[derive(Debug)]
pub enum GalleryOutput {
    Open(Open),
    /// The image was moved to the trash, or an error
    Trashed(PathBuf, Result<(), String>),
}

/// An image was chosen: the gallery's images, and the position of the chosen one
#[derive(Debug)]
pub struct Open {
    pub paths: Vec<PathBuf>,
    pub index: usize,
}

/// An image, and its thumbnail
struct Thumb {
    path: PathBuf,
    thumb: PathBuf,
}

impl RelmGridItem for Thumb {
    type Root = gtk::Picture;
    /// The image shown, for the menu
    type Widgets = Rc<RefCell<PathBuf>>;

    fn setup(_: &gtk::ListItem) -> (gtk::Picture, Self::Widgets) {
        let picture = ui::thumbnail();
        let path = Rc::<RefCell<PathBuf>>::default();
        // A menu on a right click or a long press
        let menu = |picture: &gtk::Picture, path: &Rc<RefCell<PathBuf>>, x: f64, y: f64| {
            let model = gio::Menu::new();
            let item = gio::MenuItem::new(Some("Move to Trash"), None);
            let bytes = path.borrow().as_os_str().as_bytes().to_vec();
            item.set_action_and_target_value(Some("gallery.trash"), Some(&bytes.to_variant()));
            model.append_item(&item);
            let popover = gtk::PopoverMenu::from_model(Some(&model));
            popover.set_parent(picture);
            popover.set_has_arrow(false);
            popover.set_halign(gtk::Align::Start);
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.connect_closed(|p| {
                // After the chosen item's action, which needs the popover in the tree
                let p = p.clone();
                glib::idle_add_local_once(move || p.unparent());
            });
            popover.popup();
        };
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let (p, w) = (path.clone(), picture.downgrade());
        click.connect_pressed(move |_, _, x, y| w.upgrade().iter().for_each(|w| menu(w, &p, x, y)));
        picture.add_controller(click);
        let press = gtk::GestureLongPress::builder().touch_only(true).build();
        let (p, w) = (path.clone(), picture.downgrade());
        press.connect_pressed(move |_, x, y| w.upgrade().iter().for_each(|w| menu(w, &p, x, y)));
        picture.add_controller(press);
        (picture, path)
    }

    fn bind(&mut self, path: &mut Self::Widgets, picture: &mut gtk::Picture) {
        picture.set_filename(Some(&self.thumb));
        *path.borrow_mut() = self.path.clone();
    }
}

impl Component for Gallery {
    type Init = Lib;
    type Input = GalleryMsg;
    type Output = GalleryOutput;
    type CommandOutput = ();
    type Root = gtk::Stack;
    type Widgets = GalleryWidgets;

    fn init_root() -> gtk::Stack {
        gtk::Stack::new()
    }

    fn init(lib: Lib, root: gtk::Stack, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let grid = TypedGridView::new();
        ui::tile_grid(&grid.view);
        let s = sender.clone();
        grid.view.connect_activate(move |_, position| s.input(GalleryMsg::Activated(position)));
        // The tiles' menus
        let actions = gio::SimpleActionGroup::new();
        let trash = gio::SimpleAction::new("trash", Some(glib::VariantTy::BYTE_STRING));
        trash.connect_activate(move |_, bytes| {
            if let Some(bytes) = bytes.and_then(|b| b.get::<Vec<u8>>()) {
                sender.input(GalleryMsg::Trash(std::ffi::OsString::from_vec(bytes).into()));
            }
        });
        actions.add_action(&trash);
        root.insert_action_group("gallery", Some(&actions));
        ui::add_results(&root, &ui::scrolled(&grid.view));
        let model = Gallery { lib, tag: None, query: String::new(), paths: Vec::new() };
        ComponentParts { model, widgets: GalleryWidgets { stack: root, grid } }
    }

    fn update_with_view(
        &mut self,
        w: &mut GalleryWidgets,
        msg: GalleryMsg,
        sender: ComponentSender<Self>,
        root: &gtk::Stack,
    ) {
        match msg {
            GalleryMsg::Show(tag) => (self.tag, self.query) = (tag, String::new()),
            GalleryMsg::Search(query) => self.query = query,
            GalleryMsg::Activated(position) => {
                let open = Open { paths: self.paths.clone(), index: position as usize };
                let _ = sender.output(GalleryOutput::Open(open));
                return;
            }
            GalleryMsg::Trash(path) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                ui::ask_trash(root, &name, move || sender.input(GalleryMsg::Trashed(path.clone())));
                return;
            }
            // The app has the galleries leave it out
            GalleryMsg::Trashed(path) => {
                let result = ui::trash(&mut self.lib.borrow_mut(), &path);
                let _ = sender.output(GalleryOutput::Trashed(path, result));
                return;
            }
            GalleryMsg::Removed(path) => {
                if let Some(i) = self.paths.iter().position(|p| *p == path) {
                    self.paths.remove(i);
                    w.grid.remove(i as u32);
                    // The focus goes to the next tile, not the first, which would scroll to the top
                    if let Some(next) = i.checked_sub((i == self.paths.len()) as usize) {
                        w.grid.view.scroll_to(next as u32, gtk::ListScrollFlags::FOCUS, None);
                    }
                }
                ui::show_results(&w.stack, !self.paths.is_empty());
                return;
            }
        }
        let lib = self.lib.borrow();
        self.paths = lib.paths(self.tag.as_deref(), &self.query);
        w.grid.clear();
        w.grid.extend_from_iter(
            self.paths.iter().map(|path| Thumb { path: path.clone(), thumb: lib.thumb(path) }),
        );
        ui::show_results(&w.stack, !self.paths.is_empty());
    }
}

// MARK: - Tags

type Grid = TypedGridView<Tile, gtk::NoSelection>;

pub struct Tags {
    lib: Lib,
    tags: Vec<Tag>,
    query: String,
    /// The tags of each namespace
    sections: Vec<(String, Vec<Tag>)>,
    /// The tags that match the search
    found: Vec<Tag>,
}

pub struct TagsWidgets {
    /// The sections, or the search
    stack: gtk::Stack,
    sections: gtk::Box,
    /// A row of tiles per section
    rows: Vec<Grid>,
    found: Grid,
    /// `found`, or "No Results"
    results: gtk::Stack,
}

#[derive(Debug)]
pub enum TagsMsg {
    /// Reads the tags again
    Refresh,
    Search(String),
    /// A tile of a section, or of the search
    Activated(Option<usize>, u32),
}

#[derive(Debug)]
pub enum TagsOutput {
    /// Opens the tag's gallery
    Show(String),
    /// Searches for the text
    Search(String),
}

/// A tag's cover image under a dark gradient, with its name and count
struct Tile {
    name: String,
    /// Without the namespace, where a heading names it
    label: String,
    count: usize,
    thumb: PathBuf,
}

struct TileWidgets {
    picture: gtk::Picture,
    label: gtk::Label,
    count: gtk::Label,
}

impl Tile {
    fn new(lib: &Lib, tag: &Tag, short: bool) -> Self {
        let label = if short { tag.short() } else { &tag.name };
        Tile {
            name: tag.name.clone(),
            label: format!("#{label}"),
            count: tag.count,
            thumb: lib.borrow().thumb(&tag.cover),
        }
    }
}

impl RelmGridItem for Tile {
    type Root = gtk::Overlay;
    type Widgets = TileWidgets;

    fn setup(_: &gtk::ListItem) -> (gtk::Overlay, TileWidgets) {
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
        let widgets = TileWidgets { picture: ui::thumbnail(), label: label("tag"), count: label("count") };
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::End)
            .margin_start(8)
            .margin_end(8)
            .margin_bottom(10)
            .build();
        text.append(&widgets.label);
        text.append(&widgets.count);
        let tile = gtk::Overlay::builder().child(&widgets.picture).css_classes(["tag-tile"]).build();
        tile.add_overlay(&gtk::Box::builder().css_classes(["shade"]).build());
        tile.add_overlay(&text);
        (tile, widgets)
    }

    fn bind(&mut self, widgets: &mut TileWidgets, tile: &mut gtk::Overlay) {
        widgets.picture.set_filename(Some(&self.thumb));
        widgets.label.set_label(&self.label);
        widgets.count.set_label(&self.count.to_string());
        tile.set_tooltip_text(Some(&self.name));
    }
}

/// A grid of tag tiles. `section` is its position among the sections, or `None` for the search
fn grid(section: Option<usize>, sender: &ComponentSender<Tags>) -> Grid {
    let grid = Grid::new();
    grid.view.set_single_click_activate(true);
    grid.view.add_css_class("gallery");
    let s = sender.clone();
    grid.view.connect_activate(move |_, position| s.input(TagsMsg::Activated(section, position)));
    grid
}

/// A namespace ("" for the tags without one): a heading that searches for its tags, and all of
/// them in one scrolling row
fn section(space: &str, grid: &Grid, sender: &ComponentSender<Tags>) -> gtk::Box {
    // "Creator" for `creator`
    let mut chars = space.chars();
    let title: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Other Tags".into(),
    };
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Label::builder().label(title).css_classes(["title-4"]).build());
    let heading = gtk::Button::builder()
        .child(&content)
        .halign(gtk::Align::Start)
        .margin_start(6)
        .margin_top(12)
        .css_classes(["flat"])
        .build();
    // Tags without a namespace have no prefix to search for
    if space.is_empty() {
        heading.set_can_target(false);
    } else {
        content.append(&gtk::Image::from_icon_name("go-next-symbolic"));
        let (s, prefix) = (sender.clone(), format!("{space}:"));
        heading.connect_clicked(move |_| s.output(TagsOutput::Search(prefix.clone())).unwrap());
    }
    grid.view.set_orientation(gtk::Orientation::Horizontal);
    grid.view.set_max_columns(1);
    let row =
        gtk::ScrolledWindow::builder().vscrollbar_policy(gtk::PolicyType::Never).child(&grid.view).build();
    let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
    section.append(&heading);
    section.append(&row);
    section
}

impl Component for Tags {
    type Init = Lib;
    type Input = TagsMsg;
    type Output = TagsOutput;
    type CommandOutput = ();
    type Root = gtk::Stack;
    type Widgets = TagsWidgets;

    fn init_root() -> gtk::Stack {
        gtk::Stack::new()
    }

    fn init(lib: Lib, stack: gtk::Stack, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let sections = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_bottom(12).build();
        stack.add_named(&ui::scrolled(&sections), Some("sections"));
        let found = grid(None, &sender);
        ui::tile_grid(&found.view);
        let results = gtk::Stack::new();
        ui::add_results(&results, &ui::scrolled(&found.view));
        stack.add_named(&results, Some("search"));
        let model =
            Tags { lib, tags: Vec::new(), query: String::new(), sections: Vec::new(), found: Vec::new() };
        ComponentParts { model, widgets: TagsWidgets { stack, sections, rows: Vec::new(), found, results } }
    }

    fn update_with_view(
        &mut self,
        w: &mut TagsWidgets,
        msg: TagsMsg,
        sender: ComponentSender<Self>,
        _: &gtk::Stack,
    ) {
        match msg {
            TagsMsg::Refresh => {
                self.tags = self.lib.borrow().tags();
                self.sections = library::sections(&self.tags);
                while let Some(child) = w.sections.first_child() {
                    w.sections.remove(&child);
                }
                w.rows.clear();
                for (i, (space, tags)) in self.sections.iter().enumerate() {
                    let mut grid = grid(Some(i), &sender);
                    grid.extend_from_iter(tags.iter().map(|tag| Tile::new(&self.lib, tag, true)));
                    w.sections.append(&section(space, &grid, &sender));
                    w.rows.push(grid);
                }
            }
            TagsMsg::Search(query) => self.query = query.trim().to_owned(),
            TagsMsg::Activated(section, position) => {
                let tags = section.map_or(Some(&self.found), |i| self.sections.get(i).map(|(_, tags)| tags));
                if let Some(tag) = tags.and_then(|tags| tags.get(position as usize)) {
                    sender.output(TagsOutput::Show(tag.name.clone())).unwrap();
                }
                return;
            }
        }
        self.found = library::search_tags(&self.tags, &self.query);
        w.found.clear();
        w.found.extend_from_iter(self.found.iter().map(|tag| Tile::new(&self.lib, tag, false)));
        w.stack.set_visible_child_name(if self.query.is_empty() { "sections" } else { "search" });
        ui::show_results(&w.results, !self.found.is_empty());
    }
}
