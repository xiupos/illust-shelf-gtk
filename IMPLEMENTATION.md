# Implementation Notes (supplement to the spec)

Covers only what the spec and `illust-sidecar.schema.json` leave out: decisions, pitfalls, and environment details.
Where these notes and the spec disagree, the spec wins.

## Current state

- The schema was simplified on 2026-11-11 (still `v: 1`): `creators`, `stars`, `age`, and `page` became tags (`creator:`, `rating:`, `age:`, `set:`). The app handles only `title`, `tags`, `urls`, and `note`; `added` is set when a sidecar is created; everything else (including the retired keys) is kept as unknown keys.
- `cargo test` (8 tests), `cargo build --release`, and `cargo clippy --all-targets` pass with no warnings. Non-test code is 1577 lines (model 414, main 623, detail 540).
- The app has no creator concept: `creator:` is a tag like any other. There is one Tags tab; searching the list for `creator:` (or `series:`, …) narrows it to that namespace, and tags are shown with their prefix.
- On 2026-11-11 the app was cut down further: one library folder instead of a list (`Config.folder`, chosen with `win.choose-folder` / Ctrl+O; the settings dialog is gone), no gallery diff updates, and plain `clone()` instead of `glib::clone!` with `rustfmt.toml` (`use_small_heuristics = "Max"`, `max_width = 110`). Old configs with `folders` are ignored, so the folder has to be chosen again once.
- Crates are **renamed to `gtk` (= gtk4 0.11.5) and `adw` (= libadwaita 0.9.2)**. `image` uses `default-features = false` with only `png, jpeg, webp, gif`. `serde` has `derive`.
- Not yet checked on a real display: scrolling smoothness, and launching the browser, the file manager, and the text editor (the image viewer launch was confirmed).

## Environment

- GNOME on Wayland (`DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0`). GTK 4.24.1, libadwaita 1.10.0, cargo 1.99.1, edition 2024.
- `xorg-server-xvfb` and `xdotool` are installed. Screenshots use ImageMagick's `import`. `grim`, `gnome-screenshot`, `ydotool`, and `wtype` are not available.

## Visual testing setup (set up and verified on 2026-11-10)

Run the app on a virtual X server (Xvfb) separate from the real display, take screenshots with `import`, and drive it with `xdotool`. This never touches the user's working screen.

Verified state:

- `/tmp/iv` holds the settings (`config.json`) and test images. `/tmp/iv/pics` has 6 PNGs and 1 portrait JPEG, plus these sidecars: `img2` (valid, with the unknown key `x-unknown`), `img3` (broken JSON), `img4` (`v: 2`). `/tmp/iv/many` has 3000 hard-linked 2400×1600 JPEGs for performance testing. `/tmp` is wiped on reboot; recreate with step 3 if needed.
- Helper scripts: `/tmp/iv/run.sh` launches the app on `:99` (set `BUILD=release` for the release build), and `/tmp/iv/shot.sh <name> [delay]` saves `/tmp/iv/<name>.png`.
- libadwaita rendering, Japanese text, clicks, typing with `xdotool type`, and shortcuts such as `ctrl+a` all work. The window appears at (0,0) and draws its own title bar. Clicking a widget gives it keyboard focus, so `windowfocus` is not needed.
- `MESA-EGL: warning: DRI3 error` and `Gdk-WARNING: gdk_frame_timings_submitted() called on submitted frame` appear but are harmless (environment noise from Xvfb and cairo rendering).

### 1. Install (done)

```bash
sudo pacman -S --needed xorg-server-xvfb xdotool
```

### 2. Start the virtual display

```bash
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp &
```

### 3. Prepare test folders and settings

Point the XDG directories at a temporary location so the real settings and cache stay clean. The folder chooser is hard to automate, so write the settings file first.

```bash
mkdir -p /tmp/iv/config/net.xiupos.IllustViewer /tmp/iv/cache /tmp/iv/pics
```

```bash
echo '{"folder":"/tmp/iv/pics","zoom":192}' > /tmp/iv/config/net.xiupos.IllustViewer/config.json
```

Put test images and sidecars in `/tmp/iv/pics` (generate them with `magick`).

### 4. Launch the app

```bash
DISPLAY=:99 WAYLAND_DISPLAY= GDK_BACKEND=x11 GSK_RENDERER=cairo GDK_DEBUG=no-portals XDG_CONFIG_HOME=/tmp/iv/config XDG_CACHE_HOME=/tmp/iv/cache ./target/debug/illust-viewer &
```

- `GDK_BACKEND=x11`: render to Xvfb instead of the real (Wayland) display.
- `GSK_RENDERER=cairo`: Xvfb has no GPU, so force software rendering.
- `GDK_DEBUG=no-portals`: without it, portal dialogs such as the file chooser appear on the real display.

### 5. Screenshots and input

```bash
DISPLAY=:99 import -window root /tmp/iv/shot.png
```

```bash
DISPLAY=:99 xdotool mousemove 300 300 click 1
```

```bash
DISPLAY=:99 xdotool key ctrl+plus
```

The Read tool can view the PNGs. After an action, wait about `sleep 1` for rendering before taking a screenshot.

### 6. Cleanup

```bash
pkill -x Xvfb
```

Stop the app with `pkill -x illust-viewer`. Do not use `pkill -f <pattern>`: it also matches the shell running the command and kills everything after it (this actually happened).

### Caveats

- **An instance on the real display swallows the test launch.** With the same app ID, the second instance hands off to the first and exits, so no window appears on Xvfb (and the real window gets raised). Launch the test instance on its own D-Bus session instead: `dbus-run-session -- /tmp/iv/run.sh`. To stop only the test instance, kill it by PID (check `/proc/<pid>/exe` points at `target/debug`); `pkill -x illust-viewer` would also kill the user's instance. On a private bus, `dbus-daemon` activation logs and `Gtk-CRITICAL: Unable to register the application ... org.a11y.atspi.Registry` appear; they come from the missing accessibility and systemd services and are harmless.
- Xvfb has no window manager. The window opens at the top left with its default size and cannot be maximized or resized (`xdotool windowsize` should work). If key input does not arrive, run `xdotool search --class illust-viewer windowfocus` first.
- With cairo rendering, **scrolling smoothness cannot be judged**. Have the user check the feel of synchronous thumbnail loading on the real display. This setup can verify what is shown, column count and tile size (CSS zoom), navigation, and saved results.
- To test on the real display without touching the real settings, launch normally with only `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` set.
- **Launching other apps from the test instance opens them on the real display.** D-Bus-activatable apps (Loupe, Files, …) are started by `systemd --user`, which has the real session's `DISPLAY` / `WAYLAND_DISPLAY`, even under `dbus-run-session`. Do not click the detail page's menu items, URL rows, or anything else that launches an app during automated tests.
- `Escape` on a detail page navigates back, so do not use it to close popovers during tests.
- **Start the test instance with the Bash tool's `run_in_background`** (`cd /tmp/iv && exec dbus-run-session -- /tmp/iv/run.sh`). Instances started with `&` or `setsid` inside an ordinary command are killed when that command ends. `run.sh` also sets `GTK_A11Y=none`. Header buttons move with the window width: at 1000px the main menu is at x≈857, and x≈958 is the window's close button (clicking it ends the app with exit code 0).
- In the file chooser's location field (Ctrl+L), autocompletion can append `/` to a typed file path; press End, BackSpace before Return.
- Newlines inside `xdotool type` strings do not become line breaks in a `TextView`. Type each line separately and send `xdotool key Return` between them.

## GNOME / libadwaita patterns followed

Checked against the libadwaita docs ("Adaptive Layouts", `AdwViewSwitcher`, `AdwOverlaySplitView`, `AdwSpinnerPaintable`) and the GNOME HIG (search, primary menu, utility panes) on 2026-11-11.

- **View switcher**: `AdwViewSwitcher` (policy wide) in the header; below 550sp an `AdwBreakpoint` hides it and reveals an `AdwViewSwitcherBar` at the bottom. Pages are wrapped in `AdwBreakpointBin` (360×300, same as the window minimum) because pages are rebuilt on reload, so breakpoints cannot live on the window.
- **Search**: `GtkSearchBar` under the header, toggled by a header button or Ctrl+F, type-to-search while the page is shown, live results, and a "No Results" status page when nothing matches (gallery and tag list). The placeholder is just "Search", since longer text was truncated.
- **Detail page (follows GNOME's Image Viewer, Loupe, read from its source on 2026-11-11)**: a `RaisedBorder` header (no shadow over the image) and an `AdwMultiLayoutView`: "wide" puts the details pane on the right of an `AdwOverlaySplitView` (min width 255), and below 590sp "narrow" puts it under the image in a slide-up `Revealer` (`ScrolledWindow` with natural height up to 316px, a top border line). An (i) toggle (`help-about-symbolic`, "Details"; Loupe ships its own `info-outline-symbolic`) and F9 show it; the state carries over from page to page (a thread-local, where Loupe uses a GSetting). The picture has no margin. The pane uses Loupe's spacing (24px) and, for the file, `property` rows: Folder with a flat `folder-open-symbolic` button, and Path. A fullscreen button at the header's start (`view-fullscreen-symbolic` / `view-restore-symbolic`, "Toggle Fullscreen") toggles plain window fullscreen, with F11; Escape leaves fullscreen and otherwise goes back as usual (a page shortcut that returns `Proceed` when not fullscreen). The icon is bound to the window's `fullscreened` on map and unbound on unmap, so leaving pages leaves no bindings on the window. Unlike Loupe, the header does not hide in fullscreen. No edit button, since the app has no editing. Fullscreen itself is unverified on Xvfb: there is no window manager to grant it (mutter 51 has no X11 mode). The read-only `AdwBanner` sits under the header, not inside the pane. URL rows carry the `adw-external-link-symbolic` indicator.
- **Previous / next**: Loupe's `osd-box` at the image's bottom left (12px radius, 6px padding, 18px margin, `rgba(240,240,240,.75)` / dark `rgba(0,0,0,.65)`, two flat buttons 6px apart, "Previous Image" / "Next Image"; the one with no neighbor is insensitive). Like Loupe it fades in over 200 ms on pointer motion anywhere on the page, a click on the image, or the page being shown, and fades out over 1 s after 3 s without, unless the pointer is on it or the menu is open (`adw::TimedAnimation` on opacity). Motion events with an unchanged position are ignored, since GTK sends them when widgets change under a still pointer. Loupe picks the OSD color with `@media (prefers-color-scheme: dark)`, but that did not follow libadwaita's dark style under `ADW_DEBUG_COLOR_SCHEME`, so `apply_css` in main.rs picks it from `adw::StyleManager::is_dark()` and reloads on change. Left / Right also work on the page. `win.show-item` and `win.show-sibling` take `(path, tag, query)`: the tag ("" for all) and search text of the gallery the page came from. The detail page recomputes that gallery's order with `Library::paths(tag, query)` to find its neighbors, so no list is passed around. `win.show-sibling` saves the current page, then replaces it with `nav.replace` (the rest of the stack kept), so Back still returns to the gallery in one step. Arrow keys are a `ShortcutController` on the page in the bubble phase, so text fields and lists that use them keep them.
- **Primary menu**: `open-menu-symbolic`, tooltip "Main Menu", view options first and "About Illust Viewer" (`AdwAboutDialog`) last; no Quit item. The HIG shows the primary menu only on the top-level view when navigation is hierarchical, so the detail page's menu holds only its image actions (same button, by the user's choice). No Keyboard Shortcuts item: `AdwShortcutsDialog` needs libadwaita 1.8, and the crate feature is pinned to 1.6.
- **Loading**: `AdwStatusPage` with an `AdwSpinnerPaintable` as its paintable.

## model.rs

### Loading an Item

- The sidecar path is the image path's `OsString` with `.json` appended. Do not use `with_extension` (it turns `foo.png` into `foo.json`).
- `meta` is always a JSON object.
  - No sidecar → `{"v":1}`, `editable = true`
  - Unreadable / parse failure / not an object → `{"v":1}`, `editable = false`
  - An object with `v != 1` → keep the content as read, `editable = false`
- `v` is checked with `as_u64() == Some(1)`. `1.0` does not match and becomes read-only (the safe side).
- This lets the detail page's banner pick its reason from whether `meta["v"]` is 1 (1 means "broken JSON", anything else means "unsupported version") without adding a field to `Item`.
- `Item` derives `Clone` and `PartialEq` (used by detail.rs).

### `set` pitfall

- **With `preserve_order` enabled, `Map::remove` behaves like `swap_remove` and breaks the order. Always use `shift_remove`.**
- `insert` on an existing key keeps its position; new keys go at the end.
- Values treated as removal: `None`, `Null`, an empty string, an empty array.

### Scanning

- Directories are canonicalized and checked against a visited set (handles both nested registrations and symlink loops). Files are canonicalized too and deduplicated with a `HashSet`.
- Directory checks use `fs::metadata` (follows links).
- `paths()` returns `&str`, which goes into `StringList` and GVariant `s`, so **non-UTF-8 paths are skipped while scanning**.
- Sorted by mtime descending, ties by path (to be deterministic).

### Thumbnails

- `image::open` picks the format from the extension, so use `ImageReader::open(p)?.with_guessed_format()?.decode()`.
- Generated when the thumbnail is missing or its mtime is older than the image's.
- Written to a temporary file and renamed (no half-written JPEGs during parallel generation or after interruption).
- Images smaller than 512px are not upscaled (`min(side, 512)`).
- With `to_rgb8`, transparent PNG areas tend to turn black. Composite onto white before encoding to JPEG.
- "Skip images that fail to decode" is interpreted as **excluding them from the library** (no empty tiles). Another reading (keep the item) is possible.
- EXIF orientation is not handled (no extra features).
- Measured: 3000 2400×1600 JPEGs take about 71 s on 8 cores for the first run; a cached `Library::load` takes about 10 ms.

### `Library::edit`

Signature: `edit(path, now, f)`. Steps:

1. Look up the index (`NotFound` if missing).
2. **Re-read the sidecar from disk** (same as `refresh`). Even right after an external editor or Syncthing rewrites it, broken files and other changes are not clobbered.
3. `Err` if not `editable`.
4. Clone the item as a backup, then apply `f`.
5. If nothing changed, return `Ok` without writing (no file is created either).
6. If the sidecar does not exist yet and has no `added`, set `added` to `now`. The model stays glib-free, so detail.rs passes `now` (`glib::DateTime::now_local()`, `%FT%T%:z`).
7. Save. On failure, restore the backup and return `Err`.

`create(path, now)` writes `{"v":1,"added":now}` only when the sidecar is missing (for "Edit Sidecar in Text Editor").

### Editing as text

- `Item::text(key)` gives the editor text: strings as is, lists joined with newlines.
- `Item::set_text(key, text)`: `tags` and `urls` split into lines, trimmed, empty lines dropped, duplicates removed in order; other fields are trimmed. Empty results remove the key.

### Saving

- `serde_json::to_string_pretty` + `"\n"`.
- The temporary file is named `.{sidecar file name}.tmp` (e.g. `.foo.png.json.tmp`). Starting with `.` and ending in `tmp` keeps it out of the scan and other tools. It is removed if the rename fails.

### Filtering / facets / Config

- A filter is all items or one tag string (`paths(Option<&str>, query)`). Search matches title, note, and tags (so creator names are searchable through their `creator:` tags).
- Newest first, except a `set:` tag's gallery, which is sorted by file name (ties by full path).
- `facets()`: every tag with its count and cover image (the first of its gallery: the newest, or for `set:` the first by file name), in one pass over the items. A tag counts once per item even if it repeats. Sorted by count descending, then by tag.
- `Config` has `#[serde(default)]`, and `load` falls back to `Default`. Derived `zoom` defaults to 0, so main.rs rounds anything other than 128/192/256 to 192.
- `Library::load(&[], cache)` works as an empty library (the initial value of `win.lib` at startup).

### Tests

- No crates beyond the allowed set (gtk4, libadwaita, serde, serde_json, image, rayon, and uuid for import names), so tests create a uniquely named directory under `std::env::temp_dir()`, generate small PNGs with `image`, and run `Library::load` on them. This also covers scanning, deduplication, and thumbnails.
- The schema `examples` come from `include_str!("../illust-sidecar.schema.json")`.

## main.rs

### Concurrent reloads

Ctrl+R or choosing a folder while a load is running starts a second load, so **concurrent loads can happen**. Handled without adding a field to `Win`:

- Keep the loading page in a variable. When `Library::load` returns, discard the result if `nav.visible_page()` is not that page (the newest reload wins).

### GridView

- **`max_columns` defaults to 7.** Without a large value (e.g. 64), wide windows do not get more columns.
- Column count follows the CSS `min-width` (verified: 4 columns at 256px, 8 at 128px).
- Square tiles without gaps: GridView picks the column count from the horizontal scroll policy (minimum: the CSS `min-width`, i.e. the zoom) and stretches columns to fill the row, then measures each row's height for the actual column width with the vertical scroll policy. With the default minimum policy rows are `min-height` tall and stretched tiles turn wide, so the grid sets `vscroll-policy: Natural`: a `Picture` showing a square thumbnail reports a natural height equal to its width, so every tile is a square of the column width (checked in GTK 4.24's `gtkgridview.c`, and on Xvfb at several widths). `min-height` stays as the size for a tile with no thumbnail.
- Rounded corners use `overflow: hidden` on the picture and CSS `border-radius`. The fixed rules live in the same `CssProvider` string (`apply_css`), along with the detail page's OSD and bottom pane rules.
- Synchronous thumbnail loading takes about 1.2 ms per 512px JPEG. 3000 images scroll with all tiles filled on Xvfb; smoothness is unverified.

### Rebuilding the gallery

- A gallery is filled once when its page is built, and a search change replaces the whole list with `splice`. There is no refresh on `map`: tag edits show up in a tag's gallery the next time that page is opened (scope pages are built on every push).

### SearchBar key-capture-widget

- If both the root page and a scope page capture keys from the window, **the search bar of a hidden page reacts**. Typing on a detail page would open the root search.
- Fix: in the SearchBar's `map`, `set_key_capture_widget(root)`; in `unmap`, `None`. Verified: typing on a detail page does not start the root search.
- The search toggle binds `ToggleButton::active` and `SearchBar::search-mode-enabled` bidirectionally.

### Zoom shortcuts

- The spec has only `win.zoom` (absolute), so Ctrl+Plus/Minus use `gtk::ShortcutController` + `CallbackAction` to compute the next step from the current value and activate `win.zoom`. No extra actions needed.
- Triggers: `<Control>plus|<Control>equal|<Control>KP_Add`, `<Control>minus|<Control>KP_Subtract`, `<Control>0|<Control>KP_0`.
- The menu uses radio items (`win.zoom` + `u32` target).

### Other

- `adw::ActionRow` titles are **parsed as Pango markup by default**. Rows showing tags, URLs, or paths use `use_markup(false)` (otherwise `&` or `<` breaks them).
- `activate` runs again on a second launch. If a window exists, just `present` it.
- Adding images: `win.add-images` (a `list-add-symbolic` "Add Images" button at the start of the main page's header, opening a `FileDialog::open_multiple` filtered to the supported extensions) and a `DropTarget` for `gdk::FileList` on the window both call `add_images`, which runs `model::import` for each file in `gio::spawn_blocking`, reloads, and toasts the count (with the first error, if any). `import` copies into the folder root as `<UUIDv7>.<original extension>` (`uuid` crate, `v7` feature, added with the user's approval; names sort by time added and match the user's existing files), refusing to overwrite if the name or its sidecar somehow exists, via a hidden temp file and rename, then writes `{"v":1,"added":now}`. Copies get a new mtime, so they show first. Drag and drop is unverified on Xvfb (xdotool cannot be a drag source); the dialog path was checked.
- The empty state's button and the menu item both use `win.choose-folder`. Multiple locations are handled by symlinks inside the folder; scanning follows them and skips loops and duplicates.
- `action(window, name, type, f)` in main.rs registers the plain window actions.
- Watch for reference cycles. If a closure on widget X's signal holds X or an ancestor strongly, it leaks, and every reload leaves the old root page (with its textures) behind. Hold only children and siblings strongly; downgrade the rest (e.g. `tag_grid`'s `update` holds the stack weakly, since the stack's map handler holds `update`).
- The Tags tab (`tag_grid`) is a `GridView` of tag tiles like pixiv's tag search: each tag's cover thumbnail (`Cover`), a dark gradient (`.shade`), and "#tag" (bold, up to 2 lines, ellipsized) with the count, in white. It shares `tile_grid` with the galleries (zoom, square tiles, no gaps). It is virtualized; facets are recomputed on every `map` of its stack (so edits show up when returning) and kept as a map plus order for `bind`, and a search splices the filtered tag list. A tile's tooltip is the full tag.
- `win.show-scope` takes `s` (the tag). The scope page's title is the tag itself.
- Before `nav.replace` (reload) and on the window's `close-request`, main.rs activates `detail.save` on the visible page. `app.quit` closes the window instead of quitting directly, so Ctrl+Q goes through `close-request` too.

## detail.rs

- Built once when opened (after `lib.refresh(path)` to pick up external edits). No redraws.
- The image is loaded with `gdk::Texture::from_filename`, falling back to the `image` crate (into a `MemoryTexture`) when that fails: GTK reads PNG, JPEG, and TIFF itself and other formats only through gdk-pixbuf loaders, and WebP's (`webp-pixbuf-loader`) is often not installed, while the gallery thumbnails, made with the `image` crate, showed fine. Animated GIFs show their first frame.
- Panel: read-only banner, title (`adw::EntryRow`), tags and sources as list editors, note (`gtk::TextView`), and the file rows (folder with an open button, and path).
- The header's menu (the same hamburger button as the top page: `detail::menu_button`, `open-menu-symbolic`, "Main Menu") holds only "Open in Image Viewer", "Open Containing Folder", and "Edit Sidecar in Text Editor" (`detail.open-image`, `detail.open-folder`, `detail.edit-sidecar` on the page's action group), all through `gtk::FileLauncher` (`launch` / `open_containing_folder`). Zooming is left to the image viewer instead of being built in.
- `list_editor` builds a `boxed-list`: one `adw::ActionRow` per value with a × suffix, and a last `adw::EntryRow` to add one (empty and duplicate values are ignored). The rows themselves are the state: removing or adding touches only that row, so there is no redraw and none of its focus/scroll problems. After ×, focus moves to the add row. Tag rows activate `win.show-scope`; URL rows open the browser with `gtk::UriLauncher`.
- The × handler finds its row through the widget tree (`ancestor`) instead of capturing it, so the button does not hold a reference to its own row.
- At save time the rows plus any unsubmitted text in the add row are joined with newlines and passed to `set_text`, which trims, drops empty lines, and dedupes.
- Saving is one `detail.save` action on the page's own action group. It runs on the page's `hidden` signal, and main.rs activates it before reloading and on window close. It compares each field's current text with the last saved text and passes only the changed fields to `Library::edit`, which re-reads the file first, so text editor changes to other fields survive.
- When not editable, the title row is insensitive and the text views are read-only, so nothing can change and nothing is written.
- "Edit Sidecar in Text Editor" saves pending edits first, creates the sidecar if missing, then uses `FileLauncher`. The default app for JSON is not necessarily a text editor.
- On `shown`, focus moves to the panel toggle: by default the title field gets focus with its text selected, and one stray keystroke would replace it.
- The save error toast goes through the window captured on `map`, because the page may already be detached when `hidden` fires.

## Must include in reports

- What could not be verified on real hardware (scrolling smoothness with synchronous thumbnail loading, launching the browser, the file manager, and the text editor).
- Decisions marked "interpreted" above: excluding images that fail to decode, no EXIF support, non-virtualized lists.
