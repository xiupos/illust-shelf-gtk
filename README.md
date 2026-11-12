# illust-shelf-gtk

A GNOME app for browsing and organizing illustrations in a local folder, with metadata such as tags and sources.

Metadata is stored next to each image in a sidecar JSON file: `foo.png.json` for `foo.png`. The format is [illust-sidecar](https://github.com/xiupos/illust-sidecar). Sidecars that cannot be read or have a `v` other than 1 are read-only and are never overwritten. Unknown keys are kept.

## Requirements

- Rust (with edition 2024 support)
- GTK 4.12 or later
- libadwaita 1.6 or later

Arch Linux:

```bash
sudo pacman -S --needed rust gtk4 libadwaita
```

Fedora:

```bash
sudo dnf install cargo gtk4-devel libadwaita-devel
```

Debian / Ubuntu (a release with libadwaita 1.6 or later):

```bash
sudo apt install cargo libgtk-4-dev libadwaita-1-dev
```

## Build and run

```bash
cargo build --release
```

```bash
./target/release/illust-shelf-gtk
```

Tests:

```bash
cargo test
```

## Install

Needs [Meson](https://mesonbuild.com) (`meson` in the package managers above).

```bash
meson setup build --prefix ~/.local
```

```bash
meson install -C build
```

This builds the app and installs it with its icon and its entry in the app list. Without `--prefix` it goes to `/usr/local`. `ninja -C build uninstall` removes it again.

## Data

- Settings: `~/.config/net.xiupos.IllustShelf.Gtk/config.json` (the folder and thumbnail size)
- Thumbnails: `~/.cache/net.xiupos.IllustShelf.Gtk/thumbs/`, in a folder per library (safe to delete; recreated on the next launch). Thumbnails made by earlier versions are not reused; delete the folder once to reclaim the space

## Usage

**Folder.** Choose the folder with your illustrations (Ctrl+O). Subfolders are included; for illustrations stored elsewhere, put symlinks in it. png, jpg, jpeg, webp, and gif are supported. Hidden files and `*.sync-conflict-*` files are skipped. The folder is not watched: press Ctrl+R after changing its contents.

**Adding images.** Use the + button, drop files on the window, or press Ctrl+V on the main page (copied files, or copied image data, which is saved as PNG). Images are copied into the folder under a UUIDv7 name with their extension, so names sort by when they were added, each with a sidecar holding `v` and `added`.

**Tags.** Creators, ratings, and works are plain tags with a namespace: `creator:Name`, `rating:5`, `age:general`, `set:Work`. The Tags tab has a row per namespace, most used tags first; scroll a row sideways, or click its heading to see its tags as a grid. Searching there filters tags by name. Galleries show the newest files first, except a `set:` tag's, which is in file name order.

**Details.** Click an image to edit its title, tags, source URLs, and note. Every change is saved at once.

- To add a tag or a URL, type it in the last row and press Enter. While you type a tag, the most used tags that start with it are listed below; click one to add it.
- Clicking a tag opens its gallery, and clicking a URL opens the browser. The ⋮ button of a row has Open, Copy, and Remove.
- The previous / next buttons (or Left / Right) step through the gallery you came from.
- The menu (☰) opens the image in your image viewer (for zooming), its folder, or its sidecar in a text editor.
- "Move to Trash" moves the image and its sidecar to the trash.

The sidecar is read when an image is opened and rewritten on every change, so do not edit it elsewhere while its image is open.

| Key | Action |
|---|---|
| Start typing, Ctrl+F | Search |
| Ctrl+R | Reload |
| Ctrl+Plus / Ctrl+Minus / Ctrl+0 | Thumbnail size |
| Ctrl+O | Choose folder |
| Ctrl+V | Add images from the clipboard (main page) |
| Left / Right | Previous / next image (detail page) |
| F9 | Show or hide the details pane (detail page) |
| F11 | Fullscreen (Escape leaves it) |
| Ctrl+Q | Quit |

## Other platforms

[spec/SPEC.md](spec/SPEC.md) describes the behaviour that an Illust Shelf app for another platform shares with this one, and [spec/cases.json](spec/cases.json) has test cases for it.

## License

[MIT](LICENSE)
