//! The images of a folder and their sidecars. Independent of GTK.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];
const THUMB_SIZE: u32 = 512;
/// Galleries of tags with this prefix are in file name order
const SET: &str = "set:";

/// A sidecar's content. See `illust-sidecar.schema.json`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub v: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<String>,
    /// Keys the app does not know, kept as they are
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl Default for Meta {
    fn default() -> Self {
        Meta {
            v: 1,
            title: String::new(),
            tags: Vec::new(),
            urls: Vec::new(),
            note: String::new(),
            added: None,
            extra: Default::default(),
        }
    }
}

impl Meta {
    /// Trims the texts, and drops empty and repeated tags and URLs
    fn normalize(&mut self) {
        for text in [&mut self.title, &mut self.note] {
            *text = text.trim().to_owned();
        }
        for list in [&mut self.tags, &mut self.urls] {
            let mut seen = HashSet::new();
            *list = list.iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
            list.retain(|s| seen.insert(s.clone()));
        }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    /// The image; also the item's ID
    pub path: PathBuf,
    pub meta: Meta,
    /// false if the sidecar cannot be read as a `Meta` or its `v` is not 1. It is then never written
    pub editable: bool,
}

impl Item {
    fn open(path: PathBuf) -> Self {
        let mut item = Item { path, meta: Meta::default(), editable: true };
        item.reload();
        item
    }

    fn reload(&mut self) {
        (self.meta, self.editable) = match fs::read(self.sidecar()) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (Meta::default(), true),
            Err(_) => (Meta::default(), false),
            Ok(bytes) => match serde_json::from_slice::<Meta>(&bytes) {
                Ok(meta) => {
                    let editable = meta.v == 1;
                    (meta, editable)
                }
                Err(_) => (Meta::default(), false),
            },
        };
    }

    /// `foo.png.json` for `foo.png`
    pub fn sidecar(&self) -> PathBuf {
        sidecar(&self.path)
    }

    fn matches(&self, tag: Option<&str>, query: &str) -> bool {
        let Meta { title, note, tags, .. } = &self.meta;
        let hit = |s: &String| s.to_lowercase().contains(query);
        tag.is_none_or(|tag| tags.iter().any(|t| t == tag))
            && (query.is_empty() || hit(title) || hit(note) || tags.iter().any(hit))
    }

    /// For sorting by file name
    fn by_name(&self) -> (Option<&std::ffi::OsStr>, &Path) {
        (self.path.file_name(), &self.path)
    }
}

/// A tag in use
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    /// The number of items with the tag
    pub count: usize,
    /// The first image of the tag's gallery
    pub cover: PathBuf,
}

impl Tag {
    /// The part before the first `:`, or "" if there is none
    pub fn namespace(&self) -> &str {
        self.name.split_once(':').map_or("", |(space, _)| space)
    }
}

#[derive(Default, Debug)]
pub struct Library {
    items: Vec<Item>,
    index: HashMap<PathBuf, usize>,
    cache: PathBuf,
}

impl Library {
    /// Scans `folder` and makes the missing thumbnails in `cache`. Slow, so run it off the main
    /// thread. Images that cannot be decoded are left out
    pub fn load(folder: &Path, cache: PathBuf) -> Self {
        let _ = fs::create_dir_all(&cache);
        let mut found = scan(folder);
        // Newest first
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

    fn get_mut(&mut self, path: &Path) -> io::Result<&mut Item> {
        let i =
            self.index.get(path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "image not found"))?;
        let item = &mut self.items[*i];
        match item.editable {
            true => Ok(item),
            false => Err(io::Error::new(io::ErrorKind::PermissionDenied, "the sidecar is read-only")),
        }
    }

    pub fn thumb(&self, path: &Path) -> PathBuf {
        thumb_path(&self.cache, path)
    }

    /// A gallery: the items with `tag` (all if `None`) whose title, note, or tags contain `query`,
    /// ignoring case. Newest first, except a `set:` tag's, which is in file name order
    pub fn paths(&self, tag: Option<&str>, query: &str) -> Vec<PathBuf> {
        let query = query.trim().to_lowercase();
        let mut items: Vec<&Item> = self.items.iter().filter(|it| it.matches(tag, &query)).collect();
        if tag.is_some_and(|t| t.starts_with(SET)) {
            items.sort_by_key(|it| it.by_name());
        }
        items.into_iter().map(|it| it.path.clone()).collect()
    }

    /// The tags in use, most used first, then by name
    pub fn tags(&self) -> Vec<Tag> {
        let mut tags: HashMap<&str, (usize, &Item)> = HashMap::new();
        for item in &self.items {
            for name in item.meta.tags.iter().collect::<HashSet<_>>() {
                let (count, cover) = tags.entry(name).or_insert((0, item));
                *count += 1;
                if name.starts_with(SET) && item.by_name() < cover.by_name() {
                    *cover = item;
                }
            }
        }
        let tag = |(name, (count, cover)): (&str, (usize, &Item))| Tag {
            name: name.to_owned(),
            count,
            cover: cover.path.clone(),
        };
        let mut tags: Vec<Tag> = tags.into_iter().map(tag).collect();
        tags.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        tags
    }

    /// Saves the item's sidecar as `meta`, normalized. Writes nothing if that changes nothing. A
    /// new sidecar gets `added: now`
    pub fn save(&mut self, path: &Path, mut meta: Meta, now: &str) -> io::Result<()> {
        let item = self.get_mut(path)?;
        meta.normalize();
        if meta != item.meta {
            write(item, meta, now)?;
        }
        Ok(())
    }

    /// Creates the sidecar if it is missing
    pub fn create(&mut self, path: &Path, now: &str) -> io::Result<()> {
        let item = self.get_mut(path)?;
        match item.sidecar().exists() {
            true => Ok(()),
            false => write(item, item.meta.clone(), now),
        }
    }

    /// Re-reads the item's sidecar
    pub fn refresh(&mut self, path: &Path) {
        if let Some(&i) = self.index.get(path) {
            self.items[i].reload();
        }
    }
}

fn write(item: &mut Item, mut meta: Meta, now: &str) -> io::Result<()> {
    let sidecar = item.sidecar();
    if meta.added.is_none() && !sidecar.exists() {
        meta.added = Some(now.to_owned());
    }
    write_json(&sidecar, &meta)?;
    item.meta = meta;
    Ok(())
}

/// Copies an image into `folder` as `<UUIDv7>.<ext>`, so names sort by time added, with a sidecar
/// that has only `added`. Returns the copy's path
pub fn import(folder: &Path, src: &Path, now: &str) -> io::Result<PathBuf> {
    if !is_image(src) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a supported image"));
    }
    let ext = src.extension().unwrap_or_default().to_string_lossy();
    add(folder, &ext, now, |tmp| fs::copy(src, tmp).map(drop))
}

/// Like `import`, for PNG data
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
    if dst.exists() || sidecar(&dst).exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "the name is taken"));
    }
    write_atomic(&dst, write)?;
    write_json(&sidecar(&dst), &Meta { added: Some(now.to_owned()), ..Meta::default() })?;
    Ok(dst)
}

/// `.json` appended to the whole name: `with_extension` would give `foo.png` and `foo.jpg` the
/// same sidecar
fn sidecar(image: &Path) -> PathBuf {
    let mut s = image.as_os_str().to_owned();
    s.push(".json");
    PathBuf::from(s)
}

/// The images under `folder` with their mtimes. Follows symlinks: walkdir stops at loops, and a
/// file reached twice counts once
fn scan(folder: &Path) -> Vec<(PathBuf, SystemTime)> {
    let Ok(folder) = fs::canonicalize(folder) else { return Vec::new() };
    let mut seen = HashSet::new();
    let shown = |e: &walkdir::DirEntry| {
        let name = e.file_name().to_string_lossy();
        e.depth() == 0 || !(name.starts_with('.') || name.contains(".sync-conflict-"))
    };
    let entries = walkdir::WalkDir::new(folder).follow_links(true).into_iter().filter_entry(shown);
    entries
        .flatten()
        .filter(|e| e.file_type().is_file() && is_image(e.path()))
        .filter(|e| fs::canonicalize(e.path()).is_ok_and(|real| seen.insert(real)))
        .filter_map(|e| {
            Some((e.path().to_owned(), e.metadata().ok()?.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect()
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A thumbnail is a center-cropped square JPEG in the cache folder, named by a hash of the
/// image's path
fn thumb_path(cache: &Path, image: &Path) -> PathBuf {
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, image.as_os_str().as_encoded_bytes());
    cache.join(format!("{id}.jpg"))
}

/// Makes the thumbnail if it is missing or older than the image. true if there is one to show
fn ensure_thumb(image: &Path, thumb: &Path) -> bool {
    let mtime = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    match (mtime(image), mtime(thumb)) {
        (Some(image), Some(thumb)) if thumb >= image => true,
        _ => make_thumb(image, thumb).is_ok(),
    }
}

fn make_thumb(image: &Path, thumb: &Path) -> io::Result<()> {
    // By content: the extension may be wrong
    let img = image::ImageReader::open(image)?.with_guessed_format()?.decode().map_err(io::Error::other)?;
    // Small images are not enlarged
    let size = img.width().min(img.height()).min(THUMB_SIZE);
    let square = img.resize_to_fill(size, size, image::imageops::FilterType::Triangle);

    // JPEG has no alpha: composite onto white
    let rgba = square.to_rgba8();
    let mut rgb = image::RgbImage::new(size, size);
    for (out, px) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = px[3] as u32;
        for c in 0..3 {
            out[c] = ((px[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }

    write_atomic(thumb, |tmp| {
        let mut out = io::BufWriter::new(fs::File::create(tmp)?);
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
        encoder.encode_image(&rgb).map_err(io::Error::other)?;
        out.flush()
    })
}

/// Has `write` fill a hidden temporary file next to `path` (`.<name>.tmp`, which scans skip), then
/// renames it into place
fn write_atomic(path: &Path, write: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    let result = write(&tmp).and_then(|_| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, |tmp| fs::write(tmp, text + "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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

    fn write_sidecar(image: &Path, text: &str) {
        fs::write(sidecar(image), text).unwrap();
    }

    fn read(image: &Path) -> Option<String> {
        fs::read_to_string(sidecar(image)).ok()
    }

    fn json(image: &Path) -> Value {
        serde_json::from_str(&read(image).unwrap()).unwrap()
    }

    fn load(dir: &Path) -> Library {
        Library::load(dir, dir.join(".cache"))
    }

    fn meta(lib: &Library, image: &Path) -> Meta {
        lib.get(image).unwrap().meta.clone()
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
            write_sidecar(&p, &ex.to_string());
        }
        let lib = load(&dir);
        let items: Vec<&Item> = (0..3).map(|i| lib.get(&dir.join(format!("{i}.png"))).unwrap()).collect();
        assert!(items.iter().all(|it| it.editable));
        assert_eq!(items[0].meta, Meta::default());

        let m = &items[1].meta;
        assert_eq!(m.title, "夕暮れの駅前");
        assert_eq!(m.tags.len(), 7);
        assert!(m.tags.contains(&"creator:サンプル太郎".to_owned()));
        assert_eq!(m.urls, ["https://example.com/artworks/12345678"]);
        assert_eq!(m.note, "壁紙候補");
        assert_eq!(m.added.as_deref(), Some("2026-11-03T14:11:00+09:00"));
        assert_eq!(items[2].meta.title, "Rainy Window");
    }

    #[test]
    fn save_normalizes_and_keeps_unknown_keys() {
        let dir = tmpdir("save");
        let p = dir.join("a.png");
        png(&p);
        write_sidecar(&p, r#"{"zzz": 1, "v": 1, "creators": [{"name": "A"}], "title": "a", "note": "n"}"#);
        let mut lib = load(&dir);
        let mut m = meta(&lib, &p);
        m.title = "  New  ".into();
        m.note = " \n".into();
        m.tags = ["  b ", "", "a", "b", "\t", "a  ", "c"].map(String::from).to_vec();
        m.urls = ["https://x/1", " https://x/1 ", "https://x/2"].map(String::from).to_vec();
        lib.save(&p, m, NOW).unwrap();
        // Empty fields are left out. The existing sidecar had no `added`, and saving adds none
        let expected = json!({"v": 1, "title": "New", "tags": ["b", "a", "c"],
            "urls": ["https://x/1", "https://x/2"], "zzz": 1, "creators": [{"name": "A"}]});
        assert_eq!(json(&p), expected);
        assert!(read(&p).unwrap().ends_with("}\n"));
        assert_eq!(meta(&lib, &p).tags, ["b", "a", "c"]);
        // No temporary files left behind
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(names.iter().all(|n| !n.to_string_lossy().ends_with(".tmp")));
    }

    #[test]
    fn writes_only_changes_and_sets_added_on_create() {
        let dir = tmpdir("create");
        let (new, old, third) = (dir.join("new.png"), dir.join("old.png"), dir.join("third.png"));
        for p in [&new, &old, &third] {
            png(p);
        }
        write_sidecar(&old, r#"{"v":1,"tags":["a"],"added":"2020-01-01T00:00:00Z"}"#);
        let mut lib = load(&dir);

        // Unchanged, also after normalizing: no file is created, and an existing one is left as is
        let m = Meta { title: "  ".into(), ..meta(&lib, &new) };
        lib.save(&new, m, NOW).unwrap();
        assert_eq!(read(&new), None);
        let before = read(&old).unwrap();
        lib.save(&old, meta(&lib, &old), NOW).unwrap();
        assert_eq!(read(&old).unwrap(), before);

        // Creating the sidecar records `added`; an existing `added` is kept
        let m = Meta { title: "x".into(), ..meta(&lib, &new) };
        lib.save(&new, m, NOW).unwrap();
        assert_eq!(json(&new), json!({"v": 1, "title": "x", "added": NOW}));
        let m = Meta { note: "n".into(), ..meta(&lib, &old) };
        lib.save(&old, m, "later").unwrap();
        assert_eq!(json(&old)["added"], "2020-01-01T00:00:00Z");

        // create() writes a sidecar only when there is none
        lib.create(&third, NOW).unwrap();
        assert_eq!(json(&third), json!({"v": 1, "added": NOW}));
        lib.create(&old, "later").unwrap();
        assert!(!read(&old).unwrap().contains("later"));
    }

    #[test]
    fn paths_and_tags() {
        let dir = tmpdir("query");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let [a, b, c, d] = [dir.join("a.png"), dir.join("b.PNG"), sub.join("c.png"), dir.join("d.png")];
        let (a, b, c, d) = (a.as_path(), b.as_path(), c.as_path(), d.as_path());
        for (p, t) in [(c, 500), (b, 400), (a, 300), (d, 200)] {
            png(p);
            let t = SystemTime::UNIX_EPOCH + Duration::from_secs(t);
            fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
        }
        write_sidecar(a, r#"{"v":1,"title":"Sunset","tags":["x","y","creator:Alice","set:S"]}"#);
        write_sidecar(b, r#"{"v":1,"note":"memo","tags":["x","x","creator:Alice","set:S"]}"#);
        write_sidecar(c, r#"{"v":1,"tags":["creator:Bob","set:S"]}"#);
        // Skipped
        png(&dir.join(".hidden.png"));
        png(&dir.join("f.sync-conflict-20260101-000000-ABC.png"));
        fs::write(dir.join("broken.png"), "not an image").unwrap();
        fs::write(dir.join("readme.txt"), "x").unwrap();
        // A symlink back to the root does not repeat the images
        std::os::unix::fs::symlink(&dir, sub.join("loop")).unwrap();

        let lib = load(&dir);
        let all = |q: &str| lib.paths(None, q);
        assert_eq!(all(""), [c, b, a, d]);
        assert!(lib.thumb(a).exists());
        assert_eq!(all("SUN"), [a]);
        assert_eq!(all("memo"), [b]);
        assert_eq!(all("bob"), [c]);
        assert!(all("nothing").is_empty());
        assert_eq!(lib.paths(Some("x"), ""), [b, a]);
        assert_eq!(lib.paths(Some("x"), "memo"), [b]);
        // set: galleries are in file name order instead of newest first
        assert_eq!(lib.paths(Some("set:S"), ""), [a, b, c]);

        // Counted once per item. Covers are the newest image, or for set: the first by file name
        let tag = |name: &str, count, cover: &Path| Tag { name: name.into(), count, cover: cover.into() };
        let tags = lib.tags();
        assert_eq!(
            tags,
            [
                tag("set:S", 3, a),
                tag("creator:Alice", 2, b),
                tag("x", 2, b),
                tag("creator:Bob", 1, c),
                tag("y", 1, a)
            ]
        );
        assert_eq!((tags[0].namespace(), tags[2].namespace()), ("set", ""));
    }

    #[test]
    fn read_only_sidecars_are_never_written() {
        let dir = tmpdir("readonly");
        let cases =
            [("broken", "{broken"), ("future", r#"{"v":2,"title":"future"}"#), ("float", r#"{"v":1.0}"#)];
        let cases = cases.map(|(name, text)| (dir.join(format!("{name}.png")), text));
        for (p, text) in &cases {
            png(p);
            write_sidecar(p, text);
        }
        let mut lib = load(&dir);
        for (p, text) in &cases {
            assert!(!lib.get(p).unwrap().editable);
            let m = Meta { title: "x".into(), ..meta(&lib, p) };
            assert!(lib.save(p, m, NOW).is_err());
            assert!(lib.create(p, NOW).is_err());
            assert_eq!(read(p).unwrap(), *text);
        }
        // A newer version's content is still shown
        assert_eq!(meta(&lib, &cases[1].0).title, "future");

        // refresh() picks up a sidecar broken since the library was loaded
        let fine = dir.join("fine.png");
        png(&fine);
        let mut lib = load(&dir);
        write_sidecar(&fine, "{oops");
        lib.refresh(&fine);
        assert!(lib.save(&fine, Meta { title: "x".into(), ..Meta::default() }, NOW).is_err());
        assert_eq!(read(&fine).unwrap(), "{oops");
    }

    #[test]
    fn import_copies_with_a_new_sidecar() {
        let (src, dst) = (tmpdir("import-src"), tmpdir("import-dst"));
        let a = src.join("a.JPG");
        image::RgbImage::new(4, 4).save_with_format(&a, image::ImageFormat::Jpeg).unwrap();
        write_sidecar(&a, r#"{"v":1,"title":"not copied"}"#);
        let first = import(&dst, &a, NOW).unwrap();
        let second = import(&dst, &a, NOW).unwrap();
        // <UUIDv7>.<original extension>, in the order added
        let stem = |p: &Path| p.file_stem().unwrap().to_str().unwrap().to_owned();
        for p in [&first, &second] {
            assert_eq!(uuid::Uuid::parse_str(&stem(p)).unwrap().get_version_num(), 7);
            assert_eq!(p.extension().unwrap(), "JPG");
            assert_eq!(fs::read(p).unwrap(), fs::read(&a).unwrap());
            assert_eq!(json(p), json!({"v": 1, "added": NOW}));
        }
        assert!(stem(&first) < stem(&second));
        // Not an image: nothing is copied, and no temporary files are left
        fs::write(src.join("c.txt"), "x").unwrap();
        assert!(import(&dst, &src.join("c.txt"), NOW).is_err());
        assert_eq!(fs::read_dir(&dst).unwrap().count(), 4);

        let pasted = import_png(&dst, b"png bytes", NOW).unwrap();
        assert_eq!(pasted.extension().unwrap(), "png");
        assert_eq!(fs::read(&pasted).unwrap(), b"png bytes");
    }
}
