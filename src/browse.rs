//! The lists of the top page: a gallery of thumbnails (also used for one tag's images), and the
//! Tags tab, with a row of tags per namespace or a grid of the tags that match a search.

use crate::library::Tag;
use crate::ui::{self, Lib};
use gtk::prelude::*;
use relm4::prelude::*;
use relm4::typed_view::grid::{RelmGridItem, TypedGridView};
use std::path::PathBuf;

pub struct Gallery {
    lib: Lib,
    tag: Option<String>,
    query: String,
    /// The images shown, in order
    paths: Vec<PathBuf>,
    grid: TypedGridView<Thumb, gtk::NoSelection>,
}

#[derive(Debug)]
pub enum GalleryMsg {
    /// Shows the images of the tag, or all of them, with no search
    Show(Option<String>),
    Search(String),
    Activated(u32),
}

/// An image was chosen: the gallery's images, and the position of the chosen one
#[derive(Debug)]
pub struct Open {
    pub paths: Vec<PathBuf>,
    pub index: usize,
}

struct Thumb(PathBuf);

impl RelmGridItem for Thumb {
    type Root = gtk::Picture;
    type Widgets = ();

    fn setup(_: &gtk::ListItem) -> (gtk::Picture, ()) {
        (ui::thumbnail(), ())
    }

    fn bind(&mut self, _: &mut (), picture: &mut gtk::Picture) {
        picture.set_filename(Some(&self.0));
    }
}

impl SimpleComponent for Gallery {
    type Init = Lib;
    type Input = GalleryMsg;
    type Output = Open;
    type Root = gtk::Stack;
    type Widgets = gtk::Stack;

    fn init_root() -> gtk::Stack {
        gtk::Stack::new()
    }

    fn init(lib: Lib, root: gtk::Stack, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let grid = TypedGridView::new();
        ui::tile_grid(&grid.view);
        grid.view.connect_activate(move |_, position| sender.input(GalleryMsg::Activated(position)));
        ui::add_results(&root, &ui::scrolled(&grid.view));
        let model = Gallery { lib, tag: None, query: String::new(), paths: Vec::new(), grid };
        ComponentParts { model, widgets: root }
    }

    fn update(&mut self, msg: GalleryMsg, sender: ComponentSender<Self>) {
        match msg {
            GalleryMsg::Show(tag) => (self.tag, self.query) = (tag, String::new()),
            GalleryMsg::Search(query) => self.query = query,
            GalleryMsg::Activated(position) => {
                let _ = sender.output(Open { paths: self.paths.clone(), index: position as usize });
                return;
            }
        }
        let lib = self.lib.borrow();
        self.paths = lib.paths(self.tag.as_deref(), &self.query);
        self.grid.clear();
        self.grid.extend_from_iter(self.paths.iter().map(|path| Thumb(lib.thumb(path))));
    }

    fn update_view(&self, stack: &mut gtk::Stack, _: ComponentSender<Self>) {
        ui::show_results(stack, !self.paths.is_empty());
    }
}

type Grid = TypedGridView<Tile, gtk::NoSelection>;

pub struct Tags {
    lib: Lib,
    tags: Vec<Tag>,
    query: String,
    /// A row of tags per namespace
    sections: Vec<Grid>,
    /// The tags that match the search
    found: Grid,
}

pub struct TagsWidgets {
    /// The sections, or the search
    stack: gtk::Stack,
    sections: gtk::Box,
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
        let label =
            if short { tag.name.split_once(':').map_or(&*tag.name, |(_, name)| name) } else { &tag.name };
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
        let model = Tags { lib, tags: Vec::new(), query: String::new(), sections: Vec::new(), found };
        ComponentParts { model, widgets: TagsWidgets { stack, sections, results } }
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
                // Namespaces in the order of their most used tag, tags without one last
                let mut spaces: Vec<&str> = Vec::new();
                for tag in &self.tags {
                    if !spaces.contains(&tag.namespace()) {
                        spaces.push(tag.namespace());
                    }
                }
                spaces.sort_by_key(|space| space.is_empty());
                while let Some(child) = w.sections.first_child() {
                    w.sections.remove(&child);
                }
                self.sections.clear();
                for (i, space) in spaces.into_iter().enumerate() {
                    let mut grid = grid(Some(i), &sender);
                    let tags = self.tags.iter().filter(|tag| tag.namespace() == space);
                    grid.extend_from_iter(tags.map(|tag| Tile::new(&self.lib, tag, true)));
                    w.sections.append(&section(space, &grid, &sender));
                    self.sections.push(grid);
                }
            }
            TagsMsg::Search(query) => self.query = query.trim().to_lowercase(),
            TagsMsg::Activated(section, position) => {
                let grid = section.map_or(Some(&self.found), |i| self.sections.get(i));
                if let Some(tile) = grid.and_then(|grid| grid.get(position)) {
                    sender.output(TagsOutput::Show(tile.borrow().name.clone())).unwrap();
                }
                return;
            }
        }
        let found = self.tags.iter().filter(|tag| tag.name.to_lowercase().contains(&self.query));
        self.found.clear();
        self.found.extend_from_iter(found.map(|tag| Tile::new(&self.lib, tag, false)));
        w.stack.set_visible_child_name(if self.query.is_empty() { "sections" } else { "search" });
        ui::show_results(&w.results, !self.found.is_empty());
    }
}
