# illust-viewer

A GNOME app for browsing and organizing illustrations in a local folder, along with metadata such as source and creator.

Metadata is stored in sidecar JSON files (illust-sidecar v1) next to each image. See [`illust-sidecar.schema.json`](illust-sidecar.schema.json) for the spec. The sidecar for `foo.png` is `foo.png.json`. Sidecars with broken JSON or a `v` other than 1 are treated as read-only and are never overwritten.

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
./target/release/illust-viewer
```

Tests:

```bash
cargo test
```

## Where data is stored

- Settings: `~/.config/net.xiupos.IllustViewer/config.json` (the chosen folder and thumbnail size)
- Thumbnails: `~/.cache/net.xiupos.IllustViewer/thumbs/` (safe to delete; they are recreated on the next launch)

## Usage

- On first launch, use "Choose Folder" (Ctrl+O) to pick the folder with your illustrations. Subfolders are included. To view illustrations stored elsewhere, put symlinks to them in that folder.
- To add images, use the + button at the top left of the main page or drop image files on the window. They are copied into the folder under a UUIDv7 name that keeps the extension (e.g. `01a1bb29-12c8-73cb-af91-748ad016aacc.JPG`, so names sort by when they were added), each with a new sidecar holding only `v` and `added`.
- Supported formats are png, jpg, jpeg, webp, and gif. Hidden files and `*.sync-conflict-*` files are skipped.
- On the detail page you can edit the title, tags, source URLs, and note. Tags and URLs are lists: remove one with ×, add one in the last row. Clicking a tag opens its gallery; clicking a URL opens it in the browser. Changes are saved once, when you leave the page, reload, or close the window. Fields you did not change are left as they are on disk, so you can also edit the sidecar in a text editor.
- On the detail page, the menu (☰) opens the image in your image viewer (for zooming), opens its folder in Files, or opens the sidecar in your text editor (creating it if needed).
- Creators, ratings, age ratings, and works are plain tags: `creator:Name`, `rating:1`–`rating:5`, `age:general` / `age:sensitive` / `age:questionable` / `age:explicit`, and `set:Work`. The app treats them all alike: the Tags tab shows every tag as a tile with its newest image (a `set:` shows its first page), and typing a prefix such as `creator:` in the search narrows it to that kind. A `set:` gallery is in file name order; all others show the newest files first.
- The detail page is laid out like GNOME's Image Viewer: the (i) button (or F9) shows the details pane, on the right or, in a narrow window, below the image. Moving the mouse shows previous / next buttons at the bottom left; they, or Left / Right, step to the previous or next image in the gallery you came from: search results, a tag's gallery, or a `set:` in file name order.
- When folder contents change, press Ctrl+R to reload (files are not watched).

| Key | Action |
|---|---|
| Start typing, Ctrl+F | Search |
| Ctrl+R | Reload |
| Ctrl+Plus / Ctrl+Minus / Ctrl+0 | Thumbnail size |
| Ctrl+O | Choose folder |
| Left / Right | Previous / next image (on the detail page) |
| F9 | Show or hide the details pane (the (i) button) |
| F11 | Fullscreen (on the detail page; Escape leaves it) |
| Ctrl+Q | Quit |
