//! Scanning, thumbnails, sidecar reading/writing, and filtering. Does not depend on GTK / glib.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];
const THUMB_SIZE: u32 = 512;
/// Galleries of tags with this prefix are sorted by file name
const SET: &str = "set:";

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The image. Also serves as the ID
    pub path: PathBuf,
    /// {"v":1} if there is no sidecar
    pub meta: Value,
    /// false if the JSON is broken or v != 1
    pub editable: bool,
}

impl Item {
    fn open(path: PathBuf) -> Self {
        let mut item = Item { path, meta: json!({"v": 1}), editable: true };
        item.reload();
        item
    }

    fn reload(&mut self) {
        (self.meta, self.editable) = match fs::read(self.sidecar_path()) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (json!({"v": 1}), true),
            Err(_) => (json!({"v": 1}), false),
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(meta @ Value::Object(_)) => {
                    let ok = meta.get("v").and_then(Value::as_u64) == Some(1);
                    (meta, ok)
                }
                _ => (json!({"v": 1}), false),
            },
        };
    }

    fn str(&self, key: &str) -> Option<&str> {
        self.meta.get(key)?.as_str().filter(|s| !s.is_empty())
    }

    fn list(&self, key: &str) -> Vec<&str> {
        let Some(list) = self.meta.get(key).and_then(Value::as_array) else {
            return Vec::new();
        };
        list.iter().filter_map(Value::as_str).filter(|s| !s.is_empty()).collect()
    }

    pub fn title(&self) -> Option<&str> {
        self.str("title")
    }

    pub fn note(&self) -> Option<&str> {
        self.str("note")
    }

    pub fn tags(&self) -> Vec<&str> {
        self.list("tags")
    }

    pub fn urls(&self) -> Vec<&str> {
        self.list("urls")
    }

    pub fn sidecar_path(&self) -> PathBuf {
        sidecar_path(&self.path)
    }

    /// The field as editable text. Lists are joined with newlines
    pub fn text(&self, key: &str) -> String {
        match self.meta.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(_)) => self.list(key).join("\n"),
            _ => String::new(),
        }
    }

    /// Sets a field from edited text. tags and urls take one entry per line: lines are
    /// trimmed, empty lines dropped, and duplicates removed in order. Empty fields are removed
    pub fn set_text(&mut self, key: &str, text: &str) {
        let value = if key == "tags" || key == "urls" {
            let mut seen = HashSet::new();
            let lines: Vec<&str> =
                text.lines().map(str::trim).filter(|l| !l.is_empty() && seen.insert(*l)).collect();
            json!(lines)
        } else {
            json!(text.trim())
        };
        self.set(key, value);
    }

    /// Removes the key for an empty string or array. Existing keys keep their position
    fn set(&mut self, key: &str, value: Value) {
        let Some(map) = self.meta.as_object_mut() else {
            return;
        };
        let empty = match &value {
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.is_empty(),
            _ => false,
        };
        if empty {
            // With preserve_order, remove() would swap the last key into this position
            map.shift_remove(key);
        } else {
            map.insert(key.to_owned(), value);
        }
    }

    fn matches(&self, tag: Option<&str>, query: &str) -> bool {
        let tags = self.tags();
        if tag.is_some_and(|t| !tags.contains(&t)) {
            return false;
        }
        let hit = |s: &str| s.to_lowercase().contains(query);
        query.is_empty()
            || self.title().is_some_and(hit)
            || self.note().is_some_and(hit)
            || tags.into_iter().any(hit)
    }
}

pub struct Library {
    items: Vec<Item>,
    index: HashMap<PathBuf, usize>,
    cache: PathBuf,
}

impl Library {
    /// Scans the folder (following symlinks) and generates thumbnails. Called on another
    /// thread, so it must be Send. `None` gives an empty library
    pub fn load(folder: Option<&Path>, cache: PathBuf) -> Self {
        let _ = fs::create_dir_all(&cache);
        let (mut dirs, mut seen, mut found) = (HashSet::new(), HashSet::new(), Vec::new());
        if let Some(dir) = folder.and_then(|f| fs::canonicalize(f).ok()) {
            dirs.insert(dir.clone());
            scan(&dir, &mut dirs, &mut seen, &mut found);
        }
        found.sort_by(|(a, am), (b, bm)| bm.cmp(am).then_with(|| a.cmp(b)));

        let items: Vec<Item> = found
            .into_par_iter()
            .filter(|(path, _)| ensure_thumb(path, &thumb_path(&cache, path)))
            .map(|(path, _)| Item::open(path))
            .collect();
        let index = items.iter().enumerate().map(|(i, it)| (it.path.clone(), i)).collect();
        Library { items, index, cache }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, path: &Path) -> Option<&Item> {
        self.index.get(path).map(|&i| &self.items[i])
    }

    pub fn thumb(&self, path: &Path) -> PathBuf {
        thumb_path(&self.cache, path)
    }

    /// For the gallery: all items, or those with `tag`. query is a case-insensitive
    /// substring match. Newest first, except `set:` tags, which go in file name order
    pub fn paths(&self, tag: Option<&str>, query: &str) -> Vec<&str> {
        let query = query.trim().to_lowercase();
        let mut items: Vec<&Item> = self.items.iter().filter(|it| it.matches(tag, &query)).collect();
        if tag.is_some_and(|t| t.starts_with(SET)) {
            items.sort_by(|a, b| {
                a.path.file_name().cmp(&b.path.file_name()).then_with(|| a.path.cmp(&b.path))
            });
        }
        items.into_iter().filter_map(|it| it.path.to_str()).collect()
    }

    /// For the tag list: (tag, count, cover), sorted by count descending, then by tag. The cover is
    /// the first image of the tag's gallery: the newest, or for `set:` the first by file name
    pub fn facets(&self) -> Vec<(String, usize, PathBuf)> {
        let by_name = |it: &Item| (it.path.file_name().map(ToOwned::to_owned), it.path.clone());
        let mut facets: HashMap<&str, (usize, &Item)> = HashMap::new();
        for item in &self.items {
            let tags: HashSet<&str> = item.tags().into_iter().collect();
            for tag in tags {
                let (count, cover) = facets.entry(tag).or_insert((0, item));
                *count += 1;
                if tag.starts_with(SET) && by_name(item) < by_name(cover) {
                    *cover = item;
                }
            }
        }
        let mut facets: Vec<(String, usize, PathBuf)> = facets
            .into_iter()
            .map(|(tag, (count, cover))| (tag.to_owned(), count, cover.path.clone()))
            .collect();
        facets.sort_by(|(a, an, _), (b, bn, _)| bn.cmp(an).then_with(|| a.cmp(b)));
        facets
    }

    /// Modifies and saves. Re-reads from disk first, so files broken or rewritten externally
    /// are never clobbered. Err if not editable. Writes nothing if unchanged. When this
    /// creates the sidecar, `added` is set to `now`. On failure, the in-memory item is restored
    pub fn edit(&mut self, path: &Path, now: &str, f: impl FnOnce(&mut Item)) -> io::Result<()> {
        let &i =
            self.index.get(path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "image not found"))?;
        let item = &mut self.items[i];
        item.reload();
        if !item.editable {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the sidecar is read-only"));
        }
        let before = item.clone();
        f(item);
        if *item == before {
            return Ok(());
        }
        let sidecar = item.sidecar_path();
        if !sidecar.exists() && item.meta.get("added").is_none() {
            item.set("added", json!(now));
        }
        let result = write_json(&sidecar, &item.meta);
        if result.is_err() {
            *item = before;
        }
        result
    }

    /// Creates the sidecar if it is missing (to open it in a text editor)
    pub fn create(&mut self, path: &Path, now: &str) -> io::Result<()> {
        match self.get(path) {
            Some(item) if !item.sidecar_path().exists() => {
                self.edit(path, now, |it| it.set("added", json!(now)))
            }
            _ => Ok(()),
        }
    }

    /// Re-reads only this item's JSON
    pub fn refresh(&mut self, path: &Path) {
        if let Some(&i) = self.index.get(path) {
            self.items[i].reload();
        }
    }
}

/// Copies an image into `folder` as `<UUIDv7>.<ext>` (names sort by time added), with a new
/// sidecar `{"v":1,"added":now}`. Returns the copy's path
pub fn import(folder: &Path, src: &Path, now: &str) -> io::Result<PathBuf> {
    if !is_image(src) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a supported image"));
    }
    let ext = src.extension().unwrap_or_default().to_string_lossy();
    add(folder, &ext, now, |tmp| fs::copy(src, tmp).map(drop))
}

/// Saves PNG data (a pasted image) into `folder` as `<UUIDv7>.png`, like `import`
pub fn import_png(folder: &Path, png: &[u8], now: &str) -> io::Result<PathBuf> {
    add(folder, "png", now, |tmp| fs::write(tmp, png))
}

fn add(
    folder: &Path,
    ext: &str,
    now: &str,
    write: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let dst = folder.join(format!("{}.{ext}", uuid::Uuid::now_v7()));
    // Practically never, but never overwrite
    if dst.exists() || sidecar_path(&dst).exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "the name is taken"));
    }
    let tmp = temp_path(&dst);
    if let Err(e) = write(&tmp).and_then(|_| fs::rename(&tmp, &dst)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    write_json(&sidecar_path(&dst), &json!({"v": 1, "added": now}))?;
    Ok(dst)
}

fn sidecar_path(image: &Path) -> PathBuf {
    let mut s = image.as_os_str().to_owned();
    s.push(".json");
    PathBuf::from(s)
}

fn scan(
    dir: &Path,
    dirs: &mut HashSet<PathBuf>,
    seen: &mut HashSet<PathBuf>,
    found: &mut Vec<(PathBuf, SystemTime)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.contains(".sync-conflict-") {
            continue;
        }
        let path = entry.path();
        // Non-UTF-8 paths cannot go into StringList or GVariant, so skip them
        if path.to_str().is_none() {
            continue;
        }
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if let Ok(real) = fs::canonicalize(&path)
                && dirs.insert(real)
            {
                scan(&path, dirs, seen, found);
            }
        } else if meta.is_file()
            && is_image(&path)
            && let Ok(real) = fs::canonicalize(&path)
            && seen.insert(real)
        {
            found.push((path, meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
        }
    }
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn thumb_path(cache: &Path, path: &Path) -> PathBuf {
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    cache.join(format!("{:016x}.jpg", h.finish()))
}

/// Creates the thumbnail only if missing or older than the image. true if a usable thumbnail exists
fn ensure_thumb(src: &Path, dst: &Path) -> bool {
    let mtime = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    if let (Some(s), Some(d)) = (mtime(src), mtime(dst))
        && d >= s
    {
        return true;
    }
    make_thumb(src, dst).is_ok()
}

fn make_thumb(src: &Path, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let img = image::ImageReader::open(src)?.with_guessed_format()?.decode()?;
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let size = side.min(THUMB_SIZE);
    let square = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let square = if size < side { square.thumbnail_exact(size, size) } else { square };

    // Composite transparent areas onto white
    let rgba = square.to_rgba8();
    let mut rgb = image::RgbImage::new(size, size);
    for (out, px) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = px[3] as u32;
        for c in 0..3 {
            out[c] = ((px[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }

    let tmp = temp_path(dst);
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut out = io::BufWriter::new(fs::File::create(&tmp)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&rgb)?;
        out.flush()?;
        drop(out);
        fs::rename(&tmp, dst)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// A name in the same folder that starts with `.` and ends with `.tmp`
fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    path.with_file_name(format!(".{name}.tmp"))
}

/// Writes to a temporary file, then renames. Ends with a newline
fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    let mut text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    text.push('\n');
    let tmp = temp_path(path);
    let result = fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub folder: Option<PathBuf>,
    pub zoom: u32,
}

impl Config {
    /// Defaults if unreadable
    pub fn load(path: &Path) -> Self {
        fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        write_json(path, &serde_json::to_value(self).map_err(io::Error::other)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const NOW: &str = "2026-11-11T09:00:00+09:00";

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("illust-viewer-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(dir).unwrap()
    }

    fn png(path: &Path) {
        image::RgbaImage::from_pixel(40, 30, image::Rgba([200, 100, 50, 128])).save(path).unwrap();
    }

    fn sidecar(image: &Path, text: &str) {
        fs::write(format!("{}.json", image.display()), text).unwrap();
    }

    fn read(image: &Path) -> Option<String> {
        fs::read_to_string(format!("{}.json", image.display())).ok()
    }

    fn set_mtime(path: &Path, secs: u64) {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    fn load(dir: &Path) -> Library {
        Library::load(Some(dir), dir.join(".cache"))
    }

    fn item(meta: Value) -> Item {
        Item { path: "x.png".into(), meta, editable: true }
    }

    #[test]
    fn reads_schema_examples() {
        let schema: Value = serde_json::from_str(include_str!("../illust-sidecar.schema.json")).unwrap();
        let examples = schema["examples"].as_array().unwrap();
        assert_eq!(examples.len(), 3);
        let dir = tmpdir("examples");
        for (i, ex) in examples.iter().enumerate() {
            let p = dir.join(format!("{i}.png"));
            png(&p);
            sidecar(&p, &ex.to_string());
        }
        let lib = load(&dir);
        let items: Vec<&Item> = (0..3).map(|i| lib.get(&dir.join(format!("{i}.png"))).unwrap()).collect();
        assert!(items.iter().all(|it| it.editable));
        assert_eq!(items[0].title(), None);
        assert!(items[0].tags().is_empty());

        let it = items[1];
        assert_eq!(it.title(), Some("夕暮れの駅前"));
        assert_eq!(it.tags().len(), 7);
        assert!(it.tags().contains(&"creator:サンプル太郎"));
        assert_eq!(it.urls(), ["https://example.com/artworks/12345678"]);
        assert_eq!(it.note(), Some("壁紙候補"));
        assert_eq!(it.text("urls"), "https://example.com/artworks/12345678");
        assert_eq!(it.text("added"), "2026-11-03T14:11:00+09:00");

        assert_eq!(items[2].title(), Some("Rainy Window"));
        assert!(items[2].tags().contains(&"rating:5"));
    }

    #[test]
    fn set_text_normalizes() {
        let mut it = item(json!({"v": 1, "title": "a", "tags": ["t"], "urls": ["u"], "note": "n"}));
        it.set_text("tags", "  b \n\na\n b\n\t\na  \nc");
        it.set_text("urls", "https://x/1\n https://x/1 \n\nhttps://x/2\n");
        it.set_text("title", "  New  ");
        assert_eq!(it.tags(), ["b", "a", "c"]);
        assert_eq!(it.urls(), ["https://x/1", "https://x/2"]);
        assert_eq!(it.title(), Some("New"));
        assert_eq!(it.text("tags"), "b\na\nc");

        // Empty fields are removed, keeping the other keys in place
        it.set_text("tags", " \n\n");
        it.set_text("title", "   ");
        it.set_text("note", "\n");
        assert_eq!(it.meta, json!({"v": 1, "urls": ["https://x/1", "https://x/2"]}));
    }

    #[test]
    fn keeps_unknown_keys_and_order() {
        let dir = tmpdir("order");
        let p = dir.join("a.png");
        png(&p);
        sidecar(
            &p,
            r#"{"zzz": 1, "v": 1, "creators": [{"name": "A"}], "stars": 3, "title": "a",
                "sha256": "00", "note": "n", "age": "general"}"#,
        );
        let mut lib = load(&dir);
        lib.edit(&p, NOW, |it| {
            it.set_text("title", "b");
            it.set_text("note", "");
            it.set_text("tags", "t");
        })
        .unwrap();
        let expected = json!({"zzz": 1, "v": 1, "creators": [{"name": "A"}], "stars": 3,
            "title": "b", "sha256": "00", "age": "general", "tags": ["t"]});
        let text = read(&p).unwrap();
        assert_eq!(text, serde_json::to_string_pretty(&expected).unwrap() + "\n");
        let keys: Vec<String> =
            serde_json::from_str::<Value>(&text).unwrap().as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["zzz", "v", "creators", "stars", "title", "sha256", "age", "tags"]);
        // The existing sidecar had no `added`, and editing does not add one
        assert!(!text.contains("added"));
        // No temporary files left behind
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(names.iter().all(|n| !n.to_string_lossy().ends_with(".tmp")));
    }

    #[test]
    fn writes_only_changes_and_sets_added_on_create() {
        let dir = tmpdir("create");
        let new = dir.join("new.png");
        let old = dir.join("old.png");
        png(&new);
        png(&old);
        sidecar(&old, r#"{"v":1,"tags":["a","a"],"added":"2020-01-01T00:00:00Z"}"#);
        let mut lib = load(&dir);

        // Unchanged: no file is created, and an existing one is left as is
        lib.edit(&new, NOW, |it| it.set_text("title", "")).unwrap();
        assert_eq!(read(&new), None);
        let before = read(&old).unwrap();
        lib.edit(&old, NOW, |_| {}).unwrap();
        assert_eq!(read(&old).unwrap(), before);

        // Creating the sidecar records `added`; an existing `added` is kept
        lib.edit(&new, NOW, |it| it.set_text("title", "x")).unwrap();
        assert_eq!(
            read(&new).unwrap(),
            format!("{{\n  \"v\": 1,\n  \"title\": \"x\",\n  \"added\": \"{NOW}\"\n}}\n")
        );
        lib.edit(&old, NOW, |it| it.set_text("note", "n")).unwrap();
        assert!(read(&old).unwrap().contains("2020-01-01T00:00:00Z"));

        // create() makes {"v":1,"added":…} only when missing
        let third = dir.join("third.png");
        png(&third);
        let mut lib = load(&dir);
        lib.create(&third, NOW).unwrap();
        assert_eq!(read(&third).unwrap(), format!("{{\n  \"v\": 1,\n  \"added\": \"{NOW}\"\n}}\n"));
        lib.create(&old, "later").unwrap();
        assert!(!read(&old).unwrap().contains("later"));
    }

    #[test]
    fn paths_and_facets() {
        let dir = tmpdir("query");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let a = dir.join("a.png");
        let b = dir.join("b.PNG");
        let c = sub.join("c.png");
        let d = dir.join("d.png");
        for (p, t) in [(&c, 500), (&b, 400), (&a, 300), (&d, 200)] {
            png(p);
            set_mtime(p, t);
        }
        sidecar(&a, r#"{"v":1,"title":"Sunset","tags":["x","y","creator:Alice","set:S"]}"#);
        sidecar(&b, r#"{"v":1,"note":"memo","tags":["x","x","creator:Alice","set:S"]}"#);
        sidecar(&c, r#"{"v":1,"tags":["creator:Bob","set:S"]}"#);
        // Excluded
        png(&dir.join(".hidden.png"));
        png(&dir.join("f.sync-conflict-20260101-000000-ABC.png"));
        fs::write(dir.join("broken.png"), "not an image").unwrap();
        fs::write(dir.join("readme.txt"), "x").unwrap();

        // A symlink back to the root (a loop) does not duplicate images
        std::os::unix::fs::symlink(&dir, sub.join("loop")).unwrap();
        let lib = load(&dir);
        let s = |p: &Path| p.to_str().unwrap().to_owned();
        let all = |q: &str| lib.paths(None, q);
        assert_eq!(all(""), [s(&c), s(&b), s(&a), s(&d)]);
        assert!(lib.thumb(&a).exists());
        assert_eq!(all("SUN"), [s(&a)]);
        assert_eq!(all("memo"), [s(&b)]);
        assert_eq!(all("bob"), [s(&c)]);
        assert!(all("nothing").is_empty());

        assert_eq!(lib.paths(Some("x"), ""), [s(&b), s(&a)]);
        assert_eq!(lib.paths(Some("x"), "memo"), [s(&b)]);
        assert_eq!(lib.paths(Some("creator:Alice"), ""), [s(&b), s(&a)]);
        // set: galleries go in file name order instead of newest first
        assert_eq!(lib.paths(Some("set:S"), ""), [s(&a), s(&b), s(&c)]);

        let facet = |tag: &str, count, cover: &Path| (tag.to_owned(), count, cover.to_owned());
        // Every tag, creators included, counted once per item. Covers are the newest image, or
        // for set: the first by file name
        assert_eq!(
            lib.facets(),
            [
                facet("set:S", 3, &a),
                facet("creator:Alice", 2, &b),
                facet("x", 2, &b),
                facet("creator:Bob", 1, &c),
                facet("y", 1, &a)
            ]
        );
    }

    #[test]
    fn read_only_sidecars_are_never_written() {
        let dir = tmpdir("readonly");
        let broken = dir.join("broken.png");
        let future = dir.join("future.png");
        let fine = dir.join("fine.png");
        for p in [&broken, &future, &fine] {
            png(p);
        }
        sidecar(&broken, "{broken");
        sidecar(&future, r#"{"v":2,"title":"future"}"#);
        let mut lib = load(&dir);

        for p in [&broken, &future] {
            let before = read(p).unwrap();
            assert!(!lib.get(p).unwrap().editable);
            assert!(lib.edit(p, NOW, |it| it.set_text("title", "x")).is_err());
            assert!(lib.create(p, NOW).is_ok());
            assert_eq!(read(p).unwrap(), before);
        }
        assert_eq!(lib.get(&future).unwrap().title(), Some("future"));

        // A file broken externally is re-read right before edit, so it is not clobbered
        sidecar(&fine, "{oops");
        assert!(lib.edit(&fine, NOW, |it| it.set_text("title", "x")).is_err());
        assert_eq!(read(&fine).unwrap(), "{oops");
    }

    #[test]
    fn import_copies_with_an_empty_sidecar() {
        let src = tmpdir("import-src");
        let dst = tmpdir("import-dst");
        let a = src.join("a.JPG");
        image::RgbImage::new(4, 4).save_with_format(&a, image::ImageFormat::Jpeg).unwrap();
        sidecar(&a, r#"{"v":1,"title":"not copied"}"#);
        let first = import(&dst, &a, NOW).unwrap();
        let second = import(&dst, &a, NOW).unwrap();
        // <UUIDv7>.<original extension>, in the order added
        let stem = |p: &Path| p.file_stem().unwrap().to_str().unwrap().to_owned();
        for p in [&first, &second] {
            let id = uuid::Uuid::parse_str(&stem(p)).unwrap();
            assert_eq!(id.get_version_num(), 7);
            assert_eq!(p.extension().unwrap(), "JPG");
            assert_eq!(fs::read(p).unwrap(), fs::read(&a).unwrap());
            assert_eq!(read(p).unwrap(), format!("{{\n  \"v\": 1,\n  \"added\": \"{NOW}\"\n}}\n"));
        }
        assert!(stem(&first) < stem(&second));
        // Not an image: nothing is copied, and no temporary files are left
        fs::write(src.join("c.txt"), "x").unwrap();
        assert!(import(&dst, &src.join("c.txt"), NOW).is_err());
        assert_eq!(fs::read_dir(&dst).unwrap().count(), 4);
        // Pasted PNG data is saved as <UUIDv7>.png
        let pasted = import_png(&dst, b"png bytes", NOW).unwrap();
        assert_eq!(pasted.extension().unwrap(), "png");
        assert_eq!(fs::read(&pasted).unwrap(), b"png bytes");
        assert!(read(&pasted).unwrap().contains(NOW));
    }

    #[test]
    fn config_roundtrip() {
        let dir = tmpdir("config");
        let path = dir.join("app/config.json");
        assert_eq!(Config::load(&path).zoom, 0);
        let config = Config { folder: Some("/a".into()), zoom: 256 };
        config.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.folder, config.folder);
        assert_eq!(loaded.zoom, 256);
    }
}
