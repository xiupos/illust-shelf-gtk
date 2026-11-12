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
/// The number of tags suggested while typing one
pub const SUGGESTIONS: usize = 5;

/// A sidecar's content, in the illust-sidecar format: https://github.com/xiupos/illust-sidecar
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
    /// The image's path relative to the library's folder; also the item's ID. It stays the same
    /// when the folder moves, and needs no absolute paths, which some platforms do not have
    pub path: PathBuf,
    pub meta: Meta,
    /// false if the sidecar cannot be read as a `Meta` or its `v` is not 1. It is then never written
    pub editable: bool,
}

impl Item {
    fn open(root: &Path, path: PathBuf) -> Self {
        let mut item = Item { path, meta: Meta::default(), editable: true };
        item.reload(root);
        item
    }

    fn reload(&mut self, root: &Path) {
        (self.meta, self.editable) = match fs::read(sidecar(&root.join(&self.path))) {
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

    /// The name without its namespace
    pub fn short(&self) -> &str {
        self.name.split_once(':').map_or(&self.name, |(_, name)| name)
    }
}

/// The tags by namespace, for the Tags tab. `tags` is in the order of `Library::tags`, which each
/// group keeps. Namespaces are in the order of their most used tag; the tags without one ("")
/// come last
pub fn sections(tags: &[Tag]) -> Vec<(String, Vec<Tag>)> {
    let mut sections: Vec<(String, Vec<Tag>)> = Vec::new();
    for tag in tags {
        match sections.iter_mut().find(|(space, _)| space == tag.namespace()) {
            Some((_, tags)) => tags.push(tag.clone()),
            None => sections.push((tag.namespace().to_owned(), vec![tag.clone()])),
        }
    }
    sections.sort_by_key(|(space, _)| space.is_empty());
    sections
}

/// The tags whose name contains `query`, ignoring case
pub fn search_tags(tags: &[Tag], query: &str) -> Vec<Tag> {
    let query = query.trim().to_lowercase();
    tags.iter().filter(|tag| tag.name.to_lowercase().contains(&query)).cloned().collect()
}

/// Tags to offer while `text` is typed as a new tag of an image: the first `SUGGESTIONS` of
/// `known` (every tag in use, most used first) that start with it, ignoring case, and are not
/// `attached` to the image yet
pub fn suggest(known: &[String], attached: &[String], text: &str) -> Vec<String> {
    let text = text.trim().to_lowercase();
    let fits =
        |tag: &&String| !text.is_empty() && tag.to_lowercase().starts_with(&text) && !attached.contains(tag);
    known.iter().filter(fits).take(SUGGESTIONS).cloned().collect()
}

/// Adds `value`, trimmed, to an image's tags or URLs. false if it is empty or already there
pub fn push_unique(list: &mut Vec<String>, value: &str) -> bool {
    let value = value.trim();
    let new = !value.is_empty() && !list.iter().any(|v| v == value);
    if new {
        list.push(value.to_owned());
    }
    new
}

#[derive(Default, Debug)]
pub struct Library {
    /// The folder, canonical
    root: PathBuf,
    items: Vec<Item>,
    index: HashMap<PathBuf, usize>,
    cache: PathBuf,
}

impl Library {
    /// Scans `folder` and makes the missing thumbnails in `cache`, which must be a folder of its
    /// own for each `folder`: thumbnails are named by the images' IDs. Slow, so run it off the
    /// main thread. Images that cannot be decoded are left out
    pub fn load(folder: &Path, cache: PathBuf) -> Self {
        let _ = fs::create_dir_all(&cache);
        let Ok(root) = fs::canonicalize(folder) else { return Library { cache, ..Library::default() } };
        let mut found = scan(&root);
        // Newest first
        found.sort_by(|(a, am), (b, bm)| bm.cmp(am).then_with(|| a.cmp(b)));
        let items: Vec<Item> = found
            .into_par_iter()
            .filter(|(path, _)| ensure_thumb(&root.join(path), &thumb_path(&cache, path)))
            .map(|(path, _)| Item::open(&root, path))
            .collect();
        let index = items.iter().enumerate().map(|(i, it)| (it.path.clone(), i)).collect();
        Library { root, items, index, cache }
    }

    /// The image's file
    pub fn file(&self, path: &Path) -> PathBuf {
        self.root.join(path)
    }

    /// The image's sidecar: `foo.png.json` for `foo.png`
    pub fn sidecar(&self, path: &Path) -> PathBuf {
        sidecar(&self.file(path))
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
        let sidecar = self.sidecar(path);
        let item = self.get_mut(path)?;
        meta.normalize();
        if meta != item.meta {
            write(item, &sidecar, meta, now)?;
        }
        Ok(())
    }

    /// Creates the sidecar if it is missing
    pub fn create(&mut self, path: &Path, now: &str) -> io::Result<()> {
        let sidecar = self.sidecar(path);
        let item = self.get_mut(path)?;
        match sidecar.exists() {
            true => Ok(()),
            false => write(item, &sidecar, item.meta.clone(), now),
        }
    }

    /// Re-reads the item's sidecar
    pub fn refresh(&mut self, path: &Path) {
        if let Some(&i) = self.index.get(path) {
            self.items[i].reload(&self.root);
        }
    }
}

fn write(item: &mut Item, sidecar: &Path, mut meta: Meta, now: &str) -> io::Result<()> {
    if meta.added.is_none() && !sidecar.exists() {
        meta.added = Some(now.to_owned());
    }
    write_json(sidecar, &meta)?;
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

/// The images under `folder`, which is canonical, as paths relative to it, with their mtimes.
/// Follows symlinks: walkdir stops at loops, and a file reached twice counts once
fn scan(folder: &Path) -> Vec<(PathBuf, SystemTime)> {
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
            let path = e.path().strip_prefix(folder).ok()?.to_owned();
            Some((path, e.metadata().ok()?.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect()
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A thumbnail is a center-cropped square JPEG in the cache folder, named by a hash of the
/// image's ID
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
        let dir = std::env::temp_dir().join(format!("illust-shelf-test-{}-{name}", std::process::id()));
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

    /// The ID of an image given by its file
    fn id(lib: &Library, image: &Path) -> PathBuf {
        image.strip_prefix(&lib.root).unwrap().to_owned()
    }

    fn meta(lib: &Library, image: &Path) -> Meta {
        lib.get(&id(lib, image)).unwrap().meta.clone()
    }

    #[test]
    fn reads_sidecars() {
        let dir = tmpdir("read");
        let (empty, full) = (dir.join("empty.png"), dir.join("full.png"));
        png(&empty);
        png(&full);
        write_sidecar(&empty, r#"{"v": 1}"#);
        let text = r#"{"v": 1, "title": "夕暮れの駅前", "tags": ["風景", "creator:サンプル太郎", "rating:4"],
            "urls": ["https://example.com/artworks/12345678"], "note": "壁紙候補",
            "added": "2026-11-03T14:11:00+09:00"}"#;
        write_sidecar(&full, text);
        let lib = load(&dir);
        assert!(lib.get(&id(&lib, &empty)).unwrap().editable && lib.get(&id(&lib, &full)).unwrap().editable);
        assert_eq!(meta(&lib, &empty), Meta::default());

        let m = meta(&lib, &full);
        assert_eq!(m.title, "夕暮れの駅前");
        assert_eq!(m.tags, ["風景", "creator:サンプル太郎", "rating:4"]);
        assert_eq!(m.urls, ["https://example.com/artworks/12345678"]);
        assert_eq!(m.note, "壁紙候補");
        assert_eq!(m.added.as_deref(), Some("2026-11-03T14:11:00+09:00"));
        assert!(m.extra.is_empty());
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
        lib.save(&id(&lib, &p), m, NOW).unwrap();
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
        lib.save(&id(&lib, &new), m, NOW).unwrap();
        assert_eq!(read(&new), None);
        let before = read(&old).unwrap();
        lib.save(&id(&lib, &old), meta(&lib, &old), NOW).unwrap();
        assert_eq!(read(&old).unwrap(), before);

        // Creating the sidecar records `added`; an existing `added` is kept
        let m = Meta { title: "x".into(), ..meta(&lib, &new) };
        lib.save(&id(&lib, &new), m, NOW).unwrap();
        assert_eq!(json(&new), json!({"v": 1, "title": "x", "added": NOW}));
        let m = Meta { note: "n".into(), ..meta(&lib, &old) };
        lib.save(&id(&lib, &old), m, "later").unwrap();
        assert_eq!(json(&old)["added"], "2020-01-01T00:00:00Z");

        // create() writes a sidecar only when there is none
        lib.create(&id(&lib, &third), NOW).unwrap();
        assert_eq!(json(&third), json!({"v": 1, "added": NOW}));
        lib.create(&id(&lib, &old), "later").unwrap();
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
        let files = |ids: Vec<PathBuf>| ids.iter().map(|id| lib.file(id)).collect::<Vec<_>>();
        let all = |q: &str| files(lib.paths(None, q));
        assert_eq!(all(""), [c, b, a, d]);
        assert!(lib.thumb(&id(&lib, a)).exists());
        assert_eq!(all("SUN"), [a]);
        assert_eq!(all("memo"), [b]);
        assert_eq!(all("bob"), [c]);
        assert!(all("nothing").is_empty());
        assert_eq!(lib.paths(None, "")[0], Path::new("sub/c.png"));
        assert_eq!(files(lib.paths(Some("x"), "")), [b, a]);
        assert_eq!(files(lib.paths(Some("x"), "memo")), [b]);
        // set: galleries are in file name order instead of newest first
        assert_eq!(files(lib.paths(Some("set:S"), "")), [a, b, c]);

        // Counted once per item. Covers are the newest image, or for set: the first by file name
        let tag = |name: &str, count, cover: &Path| Tag { name: name.into(), count, cover: id(&lib, cover) };
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
        assert_eq!((tags[0].short(), tags[2].short()), ("S", "x"));

        // The Tags tab: namespaces by their most used tag, the tags without one last
        let names = |tags: &[Tag]| tags.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        let sections: Vec<_> = sections(&tags).into_iter().map(|(s, tags)| (s, names(&tags))).collect();
        let section = |space: &str, tags: &[&str]| {
            (space.to_owned(), tags.iter().map(|t| t.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(
            sections,
            [
                section("set", &["set:S"]),
                section("creator", &["creator:Alice", "creator:Bob"]),
                section("", &["x", "y"])
            ]
        );
        assert_eq!(names(&search_tags(&tags, " ALI ")), ["creator:Alice"]);
        assert_eq!(search_tags(&tags, "").len(), 5);
    }

    #[test]
    fn suggests_and_adds_tags() {
        let list = |tags: &[&str]| tags.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        let known = list(&["creator:Alice", "cat", "Cb", "cc", "cd", "ce", "cf", "dog"]);
        assert_eq!(suggest(&known, &list(&["cat"]), " C"), ["creator:Alice", "Cb", "cc", "cd", "ce"]);
        assert_eq!(suggest(&known, &[], "og"), [""; 0]);
        assert_eq!(suggest(&known, &[], "  "), [""; 0]);

        let mut tags = list(&["a"]);
        assert!(push_unique(&mut tags, " b "));
        assert!(!push_unique(&mut tags, "a ") && !push_unique(&mut tags, " "));
        assert_eq!(tags, ["a", "b"]);
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
            assert!(!lib.get(&id(&lib, p)).unwrap().editable);
            let m = Meta { title: "x".into(), ..meta(&lib, p) };
            assert!(lib.save(&id(&lib, p), m, NOW).is_err());
            assert!(lib.create(&id(&lib, p), NOW).is_err());
            assert_eq!(read(p).unwrap(), *text);
        }
        // A newer version's content is still shown
        assert_eq!(meta(&lib, &cases[1].0).title, "future");

        // refresh() picks up a sidecar broken since the library was loaded
        let fine = dir.join("fine.png");
        png(&fine);
        let mut lib = load(&dir);
        write_sidecar(&fine, "{oops");
        lib.refresh(&id(&lib, &fine));
        assert!(lib.save(&id(&lib, &fine), Meta { title: "x".into(), ..Meta::default() }, NOW).is_err());
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

    /// The cases every implementation shares: see spec/SPEC.md
    #[test]
    fn spec_cases() {
        let cases: Value = serde_json::from_str(include_str!("../spec/cases.json")).unwrap();
        let now = cases["now"].as_str().unwrap();
        let strings = |v: &Value| -> Vec<String> { serde_json::from_value(v.clone()).unwrap() };

        for case in cases["sidecars"].as_array().unwrap() {
            let name = &case["name"];
            let dir = tmpdir("spec-sidecar");
            let p = dir.join("a.png");
            png(&p);
            let before = case["sidecar"].as_str().map(str::to_owned);
            if let Some(text) = &before {
                write_sidecar(&p, text);
            }
            let mut lib = load(&dir);
            let editable = case["editable"].as_bool().unwrap_or(true);
            assert_eq!(lib.get(&id(&lib, &p)).unwrap().editable, editable, "{name}");
            let mut m = meta(&lib, &p);
            let set = &case["set"];
            for (key, text) in [("title", &mut m.title), ("note", &mut m.note)] {
                if let Some(value) = set[key].as_str() {
                    *text = value.to_owned();
                }
            }
            for (key, list) in [("tags", &mut m.tags), ("urls", &mut m.urls)] {
                if !set[key].is_null() {
                    *list = strings(&set[key]);
                }
            }
            if !editable {
                m.title = "changed".into();
            }
            assert_eq!(lib.save(&id(&lib, &p), m, now).is_ok(), editable, "{name}");
            if !editable || case["unchanged"] == true {
                assert_eq!(read(&p), before, "{name}");
            } else {
                assert_eq!(json(&p), case["expect"], "{name}");
            }
        }

        let dir = tmpdir("spec-library");
        let library = &cases["library"];
        let file = |path: &Value| {
            let file = dir.join(path.as_str().unwrap());
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            png(&file.with_extension("png"));
            fs::rename(file.with_extension("png"), &file).unwrap();
            file
        };
        for f in library["files"].as_array().unwrap() {
            let p = file(&f["path"]);
            let t = SystemTime::UNIX_EPOCH + Duration::from_secs(f["mtime"].as_u64().unwrap());
            fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
            if !f["sidecar"].is_null() {
                write_sidecar(&p, &f["sidecar"].to_string());
            }
        }
        library["skipped"].as_array().unwrap().iter().for_each(|path| drop(file(path)));
        let lib = load(&dir);
        let paths = |v: &Value| strings(v).into_iter().map(PathBuf::from).collect::<Vec<_>>();
        for g in library["galleries"].as_array().unwrap() {
            let found = lib.paths(g["tag"].as_str(), g["query"].as_str().unwrap());
            assert_eq!(found, paths(&g["expect"]), "{g}");
        }
        let tags = lib.tags();
        let expected = library["tags"].as_array().unwrap().iter().map(|t| Tag {
            name: t["name"].as_str().unwrap().into(),
            count: t["count"].as_u64().unwrap() as usize,
            cover: t["cover"].as_str().unwrap().into(),
        });
        assert_eq!(tags, expected.collect::<Vec<_>>());
        let names = |tags: &[Tag]| tags.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        let found: Vec<_> = sections(&tags).into_iter().map(|(space, tags)| (space, names(&tags))).collect();
        let expected = library["sections"].as_array().unwrap().iter();
        let expected = expected.map(|s| (s["namespace"].as_str().unwrap().to_owned(), strings(&s["tags"])));
        assert_eq!(found, expected.collect::<Vec<_>>());
        for s in library["tag_searches"].as_array().unwrap() {
            assert_eq!(
                names(&search_tags(&tags, s["query"].as_str().unwrap())),
                strings(&s["expect"]),
                "{s}"
            );
        }

        let known = strings(&cases["suggestions"]["known"]);
        for s in cases["suggestions"]["cases"].as_array().unwrap() {
            let found = suggest(&known, &strings(&s["attached"]), s["text"].as_str().unwrap());
            assert_eq!(found, strings(&s["expect"]), "{s}");
        }
    }
}
