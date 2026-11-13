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

// MARK: - Meta

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

// MARK: - Item

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

// MARK: - Tags

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
        split_tag(&self.name).0
    }

    /// The name without its namespace
    pub fn short(&self) -> &str {
        split_tag(&self.name).1
    }
}

/// A tag's namespace ("" if it has none) and its name without it
pub fn split_tag(tag: &str) -> (&str, &str) {
    tag.split_once(':').unwrap_or(("", tag))
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

// MARK: - Library

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

    /// Leaves out an image that is gone, without scanning the folder again
    pub fn forget(&mut self, path: &Path) {
        if let Some(i) = self.index.remove(path) {
            self.items.remove(i);
            self.index.values_mut().filter(|j| **j > i).for_each(|j| *j -= 1);
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

// MARK: - Thumbnails

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

/// Decodes an image by its content, as the extension may be wrong, with its EXIF orientation
fn decode_oriented(path: &Path) -> io::Result<(image::DynamicImage, image::metadata::Orientation)> {
    use image::ImageDecoder;
    let mut decoder =
        image::ImageReader::open(path)?.with_guessed_format()?.into_decoder().map_err(io::Error::other)?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let img = image::DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
    Ok((img, orientation))
}

/// Decodes an image by its content, as the extension may be wrong, and turns it upright as its
/// EXIF orientation says
pub fn decode(path: &Path) -> io::Result<image::DynamicImage> {
    let (mut img, orientation) = decode_oriented(path)?;
    img.apply_orientation(orientation);
    Ok(img)
}

fn make_thumb(image: &Path, thumb: &Path) -> io::Result<()> {
    let (img, orientation) = decode_oriented(image)?;
    // Small images are not enlarged
    let size = img.width().min(img.height()).min(THUMB_SIZE);
    let mut square = img.resize_to_fill(size, size, image::imageops::FilterType::Triangle);
    // A center-cropped square turns as the whole image would, at less cost
    square.apply_orientation(orientation);

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

// MARK: - Writing

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

// MARK: - Tests

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

    /// What the cases do not say: the file's form, what is kept in memory, `create`, and `refresh`
    #[test]
    fn saves_creates_and_refreshes() {
        let dir = tmpdir("save");
        let (p, q) = (dir.join("a.png"), dir.join("b.png"));
        png(&p);
        png(&q);
        write_sidecar(&p, r#"{"v": 1, "title": "夕暮れの駅前", "added": "2020-01-01T00:00:00Z"}"#);
        let mut lib = load(&dir);
        let mut m = meta(&lib, &p);
        assert_eq!((m.title.as_str(), m.added.as_deref()), ("夕暮れの駅前", Some("2020-01-01T00:00:00Z")));
        m.tags = vec![" a ".into()];
        lib.save(&id(&lib, &p), m, NOW).unwrap();
        assert_eq!(meta(&lib, &p).tags, ["a"]);
        assert!(read(&p).unwrap().ends_with("}\n"));
        // No temporary files left behind
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(names.iter().all(|n| !n.to_string_lossy().ends_with(".tmp")));

        // create() writes a sidecar only when there is none
        lib.create(&id(&lib, &q), NOW).unwrap();
        assert_eq!(json(&q), json!({"v": 1, "added": NOW}));
        lib.create(&id(&lib, &p), "later").unwrap();
        assert!(!read(&p).unwrap().contains("later"));

        // refresh() picks up a sidecar changed since the library was loaded. A newer version's
        // content is shown, and not written
        let text = r#"{"v":2,"title":"future"}"#;
        write_sidecar(&q, text);
        lib.refresh(&id(&lib, &q));
        let m = meta(&lib, &q);
        assert_eq!(m.title, "future");
        assert!(lib.save(&id(&lib, &q), Meta { title: "x".into(), ..m }, NOW).is_err());
        assert!(lib.create(&id(&lib, &q), NOW).is_err());
        assert_eq!(read(&q).unwrap(), text);
    }

    #[test]
    fn scans_through_symlinks_and_skips_broken_images() {
        let dir = tmpdir("scan");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        png(&sub.join("a.png"));
        fs::write(dir.join("broken.png"), "not an image").unwrap();
        // A symlink back to the root does not repeat the images
        std::os::unix::fs::symlink(&dir, sub.join("loop")).unwrap();
        let lib = load(&dir);
        assert_eq!(lib.paths(None, ""), [Path::new("sub/a.png")]);
        assert!(lib.thumb(Path::new("sub/a.png")).exists());
    }

    #[test]
    fn forgets_images() {
        let dir = tmpdir("forget");
        for name in ["a.png", "b.png", "c.png"] {
            png(&dir.join(name));
        }
        let mut lib = load(&dir);
        lib.forget(Path::new("b.png"));
        lib.forget(Path::new("missing.png"));
        let mut paths = lib.paths(None, "");
        paths.sort();
        assert_eq!(paths, [Path::new("a.png"), Path::new("c.png")]);
        assert!(lib.get(Path::new("b.png")).is_none() && lib.get(Path::new("c.png")).is_some());
    }

    #[test]
    fn turns_images_upright() {
        let dir = tmpdir("orientation");
        // Red on the left, blue on the right, in a JPEG whose EXIF says to turn it a quarter
        // clockwise: red is then on top
        let img = image::RgbImage::from_fn(40, 20, |x, _| match x < 20 {
            true => image::Rgb([255, 0, 0]),
            false => image::Rgb([0, 0, 255]),
        });
        // A big-endian TIFF header and one IFD entry: Orientation (0x0112), SHORT, 6
        let exif = b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0".to_vec();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            fs::File::create(dir.join("a.jpg")).unwrap(),
            95,
        );
        image::ImageEncoder::set_exif_metadata(&mut encoder, exif).unwrap();
        encoder.encode_image(&img).unwrap();

        let upright = decode(&dir.join("a.jpg")).unwrap();
        assert_eq!((upright.width(), upright.height()), (20, 40));
        let thumb = image::open(load(&dir).thumb(Path::new("a.jpg"))).unwrap().to_rgb8();
        let (top, bottom) = (thumb.get_pixel(10, 2), thumb.get_pixel(10, 17));
        assert!(top[0] > 200 && top[2] < 50, "{top:?}");
        assert!(bottom[2] > 200 && bottom[0] < 50, "{bottom:?}");
    }

    #[test]
    fn adds_tags() {
        let mut tags = vec!["a".to_owned()];
        assert!(push_unique(&mut tags, " b "));
        assert!(!push_unique(&mut tags, "a ") && !push_unique(&mut tags, " "));
        assert_eq!(tags, ["a", "b"]);
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
