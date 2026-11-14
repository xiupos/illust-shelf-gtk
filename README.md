# illust-shelf-gtk

A GNOME app for browsing and tagging illustrations in a local folder.

Metadata is kept next to each image in a sidecar JSON file (`foo.png.json` for `foo.png`), in the [illust-sidecar](https://github.com/xiupos/illust-sidecar) format.

## Build

Needs Rust (edition 2024), GTK 4.12+, and libadwaita 1.6+.

```bash
# Arch
sudo pacman -S --needed rust gtk4 libadwaita

# Fedora
sudo dnf install cargo gtk4-devel libadwaita-devel

# Debian / Ubuntu
sudo apt install cargo libgtk-4-dev libadwaita-1-dev
```

```bash
cargo run --release
```

To install it with its icon and desktop entry, use [Meson](https://mesonbuild.com):

```bash
meson setup build --prefix ~/.local && meson install -C build
```

`ninja -C build uninstall` removes it again, as long as the `build` folder is kept.

## Usage

Choose a folder (Ctrl+O). Its subfolders and symlinks are included; png, jpg, webp, and gif are shown. The folder is not watched, so press Ctrl+R after changing it.

- **Add images** with the + button, by dropping files, or with Ctrl+V. They are copied into the folder.
- **Tags** with a namespace, like `creator:Name` or `set:Work`, are grouped by namespace in the Search tab, before anything is typed.
- **Click an image** to edit its title, tags, source URLs, and note. Changes are saved at once.
- **Move an image to the trash** with Delete, or by right-clicking its thumbnail. Its sidecar goes with it.

| Key | Action |
|---|---|
| Ctrl+F | Search (on a tag's page, also start typing) |
| Ctrl+R | Reload |
| Ctrl+Plus / Ctrl+Minus / Ctrl+0 | Thumbnail size |
| Ctrl+O | Choose folder |
| Ctrl+V | Add images from the clipboard |
| Left / Right | Previous / next image |
| Delete | Move the image to the trash |
| F9 | Show or hide the details pane |
| F11 | Fullscreen |
| Ctrl+Q | Quit |

Settings are in `~/.config/net.xiupos.IllustShelf.Gtk/`, thumbnails in `~/.cache/net.xiupos.IllustShelf.Gtk/` (safe to delete).

## Other platforms

- [illust-shelf-ios](https://github.com/xiupos/illust-shelf-ios): Illust Shelf for iPhone and iPad

[spec/SPEC.md](spec/SPEC.md) describes the behaviour these apps share, with test cases in [spec/cases.json](spec/cases.json).

## License

[MIT](LICENSE)
