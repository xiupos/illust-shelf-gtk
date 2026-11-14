# Illust Shelf: Shared Behaviour

What every Illust Shelf app does, whatever its platform. The reference implementation is `src/library.rs`. Rules marked **(cases)** have test cases in [`cases.json`](cases.json); see [Test cases](#test-cases). Anything not said here is up to the implementation.

## Library

A library is one folder, chosen by the user, and the images under it, at any depth.

- **Images** are the files with the extension `png`, `jpg`, `jpeg`, `webp`, or `gif`, in any case. An implementation may support more formats.
- **Skipped (cases):** files and folders whose name starts with `.`, and those whose name contains `.sync-conflict-` (Syncthing's copies).
- An image that cannot be decoded may be left out.
- Where the platform has symlinks, they are followed, loops end, and a file reached twice counts once.
- The folder is not watched. The user reloads it.
- **ID.** An image is identified by its path relative to the folder, with `/` between the parts, never by an absolute path (some platforms have none).

## Sidecars

An image's metadata is in a JSON file next to it, named by appending `.json` to the image's whole name: `foo.png.json` for `foo.png` (not `foo.json`, which `foo.jpg` would share).

The format is [illust-sidecar](https://github.com/xiupos/illust-sidecar), version 1. The app edits `title`, `tags`, `urls`, and `note`, and sets `added`.

### Reading (cases)

| The sidecar | The image |
|---|---|
| is missing | has empty metadata and is editable |
| is version 1 | has its metadata and is editable |
| cannot be read, is not JSON, has a known key of the wrong type, or has a `v` that is not the integer 1 | is read-only |

A read-only image is shown, with whatever could be read, and its sidecar is never written. The sidecar is read again each time the image is opened.

### Saving (cases)

Every edit is saved at once; there is no save button.

1. Trim `title` and `note`. Trim each tag and URL, drop the empty ones, then drop the repeats, keeping the first.
2. If the result equals what was read, write nothing. In particular, no sidecar is created for an image without metadata.
3. Otherwise write the whole sidecar:
   - `v` is 1.
   - Empty fields (`""`, `[]`) are left out.
   - Keys the app does not know are kept as they were.
   - `added` is set to the current local time, in ISO 8601 with an offset (`2026-11-11T09:00:00+09:00`), only when the write creates the sidecar. Otherwise it is kept as it was, also if missing.
   - The file is indented JSON, ends with a newline, and is replaced atomically (written to a temporary file, then renamed). The temporary file's name starts with `.`, so that a scan skips it.

Changes made to the sidecar elsewhere since the image was opened are overwritten.

## Galleries (cases)

A gallery lists images: all of them, or those with one tag.

- **Order.** Newest modification time first, then by ID. A gallery of a tag that starts with `set:` is instead in file name order, then by ID.
- **Search.** The text is trimmed. An image matches if its title, its note, or one of its tags contains the text, ignoring case. An empty text matches all.
- Editing an image does not reorder or refilter a gallery that is open.

## Tags

A tag is a string. The part before its first `:`, if any, is its **namespace**: `creator` for `creator:Name`. Creators, ratings, and works are plain tags with a namespace.

- **The tags in use (cases)**, each with the number of images that have it (an image counts once), and a cover: the first image of the tag's gallery. They are ordered by that number, largest first, then by name.
- **Sections (cases).** Tags are grouped by namespace, in the order of each namespace's most used tag, with the tags without one last. Inside a section, tags keep their order and are shown without the namespace. The heading is the namespace, capitalized.
- **Search (cases).** The text is trimmed. A tag matches if its name contains the text, ignoring case. Choosing a section's heading searches for `<namespace>:`.
- **Suggestions (cases).** While the user types a new tag for an image, the app offers the tags in use that start with the trimmed text, ignoring case, and are not on the image yet: the first 5, most used first. Nothing is offered for an empty text.
- **Adding** a tag or a URL trims it, and does nothing if it is empty or already on the image.
- The app must not change a field's text while the user types in it, since that breaks input methods. Suggestions are chosen, not completed into the field.

## Adding images

An image is copied into the library's folder, not moved or linked.

- The copy is named `<UUIDv7>.<extension>`, with the extension as it was, so that names sort by the time they were added.
- It gets a sidecar with only `v` and `added`.
- Image data without a file (a clipboard, a share sheet) is saved as PNG.
- A file that is not an image by its extension is refused.

## Removing images

An image is removed together with its sidecar, after the user confirms. Where the platform has a trash, they go there.

## Screens

Three screens, each of which exists once:

1. **Top**: all images, and a search. Before anything is typed, the search shows the tags by section; then the tags and the images that match.
2. **A tag's gallery**, with a search.
3. **An image**: the image, previous / next through the gallery it was opened from, and its title, tags, URLs, and note to edit. Choosing a tag here opens that tag's gallery in place of this screen.

Otherwise, follow the platform's conventions.

## Left to the implementation

Thumbnails (the reference makes 512 px center-cropped JPEGs while loading; on devices with less memory, make them as they come into view), EXIF orientation (the reference turns images upright, thumbnails too), access to the folder, settings, opening files and URLs in other apps, and input such as keys, gestures, and drag and drop.

## Test cases

`cases.json` is independent of any language. `src/library.rs` (`spec_cases`) shows how to run it.

- `now`: the time to use for `added`.
- `sidecars`: each case is a folder with one image.
  - `sidecar`: the text of its sidecar, or `null` for none.
  - `editable`: whether the image is editable. `true` if left out.
  - `set`: the fields the user changes to the given values. Then the image is saved.
  - `expect`: the sidecar afterwards, compared as JSON.
  - `unchanged`: instead of `expect`, the sidecar's text is exactly as before, or there is still none.
  - If the image is not editable, try to change it anyway: saving fails and the text is as before.
- `library`: one folder.
  - `files`: the images, with their modification time in seconds since 1970 and their sidecar, if any.
  - `skipped`: more files, all valid images, that must not show up.
  - `galleries`: the IDs listed for a tag (`null` for all images) and a search text.
  - `tags`: the tags in use.
  - `sections`: the namespaces and the tags of each.
  - `tag_searches`: the names of the tags found.
- `suggestions`: with `known` as the tags in use, the tags offered for `text` on an image that has the tags `attached`.
