# Implementation Notes

What the code does not say by itself: the reasons behind decisions, pitfalls, and how to test the UI.

## Layout

The UI is built with [Relm4](https://relm4.org) components, which talk through messages.

| File | Contents |
|---|---|
| `src/main.rs` | The window (`App`): pages and navigation, zoom, loading, adding images. Also the settings file |
| `src/browse.rs` | `Gallery`, a grid of thumbnails, and `Search`, the top page's search: the tags by section, then the tags and images that match |
| `src/detail.rs` | `Detail`: one image and the pane that edits its sidecar |
| `src/ui.rs`, `src/style.css` | Shared widgets, and the CSS |
| `src/library.rs` | The images of a folder and their sidecars: scanning, thumbnails, filtering, saving, importing, and the rules for tags (sections, search, suggestions). No GTK. Has the tests |
| `spec/` | `SPEC.md`: the behaviour shared with the apps for other platforms. `cases.json`: test cases for it, in no particular language, which `cargo test` runs |
| `data/`, `meson.build` | The icons, the desktop entry, and the AppStream metadata. Meson installs them, and has cargo build the app |

Crates are renamed: `gtk` is gtk4 and `adw` is libadwaita, at the versions relm4 uses. Check with `cargo test`, `cargo clippy --all-targets`, and `cargo fmt`.

## Library

- **Shared behaviour** belongs in `library.rs`, with a test, and in `spec/SPEC.md` (with cases in `spec/cases.json` where it has them). Other platforms have nothing else to go by. The components only show state and pass on what the user does.
- Rules already in SPEC.md (IDs, sidecars, saving, order) are not repeated here. Sidecars are read into `Meta`; unknown keys go to `extra`. `added`'s time comes from the caller, which keeps the library free of glib.
- **Scanning** (`walkdir`) follows symlinks; files are deduplicated by canonical path.
- **Thumbnails**: named by a UUIDv5 of the image's ID, in a cache folder per library named by a UUIDv5 of its path (`main.rs`), remade when older than the image. Images that fail to decode are left out. `decode` turns images upright by their EXIF orientation, for thumbnails and the detail page; a thumbnail is turned after it is cropped, which costs less and gives the same square.
- Timing: a thumbnail of a 2400×1600 JPEG takes about 18 ms on 8 cores; a cached load of 3000 images about 10 ms.

## UI

- **Components** are written one way. The model holds state only; widgets live in the component's `Widgets`. The view is built in `init` with builders and small functions, not with the `view!` macro. `update_with_view` changes the state and then the widgets that show it. Lists (tag rows, suggestions, the search's sections) are rebuilt from the state instead of being edited in place.
- **Pages.** There are three, each built once: the top page, a tag's gallery, and the detail page. `App` alone moves between them; the components ask through their outputs. Opening a tag from the detail page replaces it with the tag's page.
- **The library** is one `Rc<RefCell<Library>>` shared by all components. A reload replaces its content, so the components' handles stay valid. Trashing an image only `forget`s it, without a reload. Loading runs as a command; `generation` drops the result of a load that a newer one has overtaken.
- **Saving.** The detail page saves on every change, so nothing is pending when a page or the window closes. Filling the fields in from code also emits `changed`; `loading` tells those apart from edits.
- **Input methods.** The app never changes the text of a field while the user types in it: that resets the input method and can commit a half-typed preedit. Tag suggestions are therefore rows to click, and the add rows are cleared only after Enter. Check changes here with an input method (below).
- **The top page** has two tabs, as on iOS: Library (all images, never searched, so it keeps its place) and Search, whose entry is always shown. Search shows the tags by section until something is typed, then the tags and the images that match, in a gallery of its own. Escape empties the search, back to the tags, and takes the focus out of the entry; it stays on the tab. Typing on Library does not search: the entry, hidden on its tab, would have to capture keys and take the focus. The tags are refreshed when tags change. The images of a matching tag match too, so only the gallery shows "No Results".
- **Galleries are not refreshed** by edits, only by a search, a reload, or being shown for another tag, so they keep their scroll position.
- **Trashing** (the detail page, Delete, or a tile's menu) goes through `ui::ask_trash` and ends in `App`'s `Msg::Trashed`: the galleries drop the tile in place (`Removed`) and the detail page goes back. The focus moves to the next tile: GTK would otherwise focus the first and scroll to the top. Tiles' menus open on a right click or a long press, with the path as the action's target.
- **Keys.** Those of the detail page (F9, Left / Right, Delete) and of the top page (Ctrl+V, Ctrl+F) are on the window, through `page_shortcut`: with nothing focused, key events do not pass through the page. Escape is on the page, since the navigation view would handle it before the window.
- **The pane** has the order of the iOS app's Info. Its dimensions come from the decoded image, so they are upright and appear once it is decoded.
- **Images** are decoded with the `image` crate, off the main thread: GTK needs a gdk-pixbuf loader for WebP, which is often missing. A 2400×1600 image takes 35 ms (JPEG) to 100 ms (PNG). Animated GIFs show their first frame.
- **Pasting.** Another app's clipboard lists only MIME types, hence `union_deserialize_types()`.

## Testing the UI

Run the app on a virtual X server, so that it never touches the real screen. Needs `xorg-server-xvfb`, `xdotool`, and ImageMagick.

```bash
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp &
mkdir -p /tmp/iv/config/net.xiupos.IllustShelf.Gtk /tmp/iv/cache /tmp/iv/pics
echo '{"folder":"/tmp/iv/pics","zoom":192}' > /tmp/iv/config/net.xiupos.IllustShelf.Gtk/config.json
# put test images and sidecars in /tmp/iv/pics, then:
dbus-run-session -- env GTK_A11Y=none DISPLAY=:99 WAYLAND_DISPLAY= GDK_BACKEND=x11 GSK_RENDERER=cairo GDK_DEBUG=no-portals XDG_CONFIG_HOME=/tmp/iv/config XDG_CACHE_HOME=/tmp/iv/cache ./target/debug/illust-shelf-gtk
```

- `dbus-run-session`: with the same app ID on the same bus, a second instance hands off to the user's running one and exits.
- `GSK_RENDERER=cairo`: Xvfb has no GPU.
- `GDK_DEBUG=no-portals`: otherwise file dialogs open on the real display.

Drive it with `DISPLAY=:99 xdotool mousemove 300 300 click 1`, and look with `DISPLAY=:99 import -window root /tmp/iv/shot.png`.

Caveats:

- Take a screenshot before clicking by coordinates: rows move as tags and URLs are added.
- Stop the test instance by PID, after checking that `/proc/<pid>/exe` is the build under `target/`. `pkill -x illust-shelf-gtk` also kills the user's instance, and `pkill -f` kills the shell that runs it.
- Do not click anything that launches another app (the detail page's menu, URL rows): it opens on the real display.
- There is no window manager: the window sits at (0, 0) with its default size (1100×760), and fullscreen cannot be tested. `xdotool windowsize` resizes it.
- `Escape` on the detail page goes back; do not use it to close popovers.
- `xdotool type` does not turn newlines into line breaks in a `TextView`; send `key Return`.
- Warnings about DRI3, frame timings, and the accessibility bus are noise.
- GIO does not trash files on `/tmp`. Test the trash in a folder under the home directory, with `XDG_DATA_HOME` pointing next to it.
- Input methods: start a separate `fcitx5 --disable=wayland,notificationitem -r` on `:99` with a copy of the user's fcitx5 config, and the app with `GTK_IM_MODULE=fcitx`, in the same `dbus-run-session`. `xdotool type --delay 5` reproduces fast typing. The input mode is per field.

## Not verified

Xvfb cannot show these; check them on a real display:

- Scrolling smoothness (thumbnails load synchronously, about 1.2 ms each)
- Fullscreen, and drag and drop
- The file dialogs, and launching the browser, the file manager, and the text editor
