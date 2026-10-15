//! Vanillish: **copying the game data in from a folder outside the app** — on Android, a folder the
//! player picks with the Storage Access Framework (this phone's storage, a USB-C stick or drive, a
//! network share behind a provider app). Such a folder has no path the app can open: the Java side
//! lists it and hands over one file descriptor per archive, so the import is a **copy, never a
//! move**, and the original stays where it was.
//!
//! Everything here is plain logic over two seams — a [`Listing`] of the picked folder and an
//! [`Open`] that reads one listed file — so it is tested on any machine with a folder standing in
//! for the provider:
//!
//! - [`candidates`] groups a listing into the folders that hold archives (the rule
//!   [`crate::data::find_imports`] applies to a path), complete ones first;
//! - [`copy_in`] copies one candidate's archives into the data folder: an archive already there at
//!   its size is kept, a `.<name>.part` from an earlier attempt resumes, the copy goes in 4 MiB
//!   chunks with Pause checked between them, and every file's size and MPQ header are checked
//!   before it takes its final name — so an interrupted copy (Pause, a crash, the process killed in
//!   the background) never leaves a half archive benilla would open;
//! - [`mpq_looks_whole`], [`settled`] and [`rename_works`] are the guards the boot-time import of a
//!   folder copied in over USB applies before it moves anything ([`launch_guard`], from
//!   `boot::import_from_documents`):
//!   that move runs before the app exists, where a copy or a folder a PC is still writing into must
//!   never be taken.
//!
//! Nothing here is Android's: iOS could use the same module behind a folder document picker.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use crate::data::{self, DataStatus, Progress, Step, EXPECTED, REQUIRED};

/// How deep a picked folder is searched, as [`crate::data::find_imports`] searches Documents:
/// `Data/`, `World of Warcraft/Data/` and `WoW 1.12.1/World of Warcraft/Data/` are all found.
pub const MAX_DEPTH: u32 = 3;

/// The most folders a listing visits before it gives up and says so (`truncated`): a picked
/// folder that is not a client (the whole of Download, say) must not be walked for minutes.
pub const MAX_FOLDERS: usize = 400;

/// Folders a listing never descends into (compared case-insensitively): a client's big trees that
/// hold no archive benilla mounts — addons, saved settings, caches, screenshots, logs.
pub const SKIP_DIRS: &[&str] = &["interface", "wtf", "cache", "screenshots", "logs", "errors"];

/// The copy's unit: Pause is honoured and progress reported between chunks.
pub const CHUNK: u64 = 4 << 20;

/// The read buffer inside a chunk.
const BUF: usize = 1 << 20;

/// One file of a picked folder, as the picker lists it (the Java side's JSON, field for field).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Listed {
    /// Relative to the picked folder, `/`-separated: `Data/dbc.MPQ`.
    pub path: String,
    pub size: u64,
    /// Last modified, in milliseconds since the epoch (`0` when the provider does not say).
    #[serde(rename = "modified", default)]
    pub modified_ms: i64,
    /// What [`Open`] opens it by: the provider's document id (on the desktop, the file's path).
    #[serde(rename = "doc")]
    pub id: String,
}

impl Listed {
    /// The file's name, the last part of its path.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The folder it is in, relative to the picked one (`""` for the picked folder itself).
    pub fn folder(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |(dir, _)| dir)
    }
}

/// A picked folder's archives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Listing {
    /// The picked folder's name, for the player ("World of Warcraft").
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub files: Vec<Listed>,
    /// The walk stopped at [`MAX_FOLDERS`]: an archive deeper in may have been missed.
    #[serde(default)]
    pub truncated: bool,
}

impl Listing {
    /// The Java side's listing JSON.
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("the folder listing did not parse: {e}"))
    }
}

/// The picked folder whose copy has not finished, as the Java side keeps it across a restart:
/// `{"tree": "content://…", "label": "World of Warcraft"}` → `(tree, label)`.
pub fn parse_pending(json: &str) -> Option<(String, String)> {
    #[derive(Deserialize)]
    struct Pending {
        tree: String,
        #[serde(default)]
        label: String,
    }
    let p: Pending = serde_json::from_str(json).ok()?;
    (!p.tree.is_empty()).then_some((p.tree, p.label))
}

/// A folder in a listing that holds mountable archives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderCandidate {
    /// Relative to the picked folder, `/`-separated (`""` = the picked folder itself).
    pub dir: String,
    /// What it holds, as [`crate::data::status`] would say of it on disk.
    pub status: DataStatus,
    /// Its mountable archives, as listed.
    pub files: Vec<Listed>,
}

impl FolderCandidate {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Group a listing by folder: every folder with mountable, non-empty archives directly in it is a
/// candidate — complete ones first, then by archive count, then the shallowest.
pub fn candidates(files: &[Listed]) -> Vec<FolderCandidate> {
    let mut out: Vec<FolderCandidate> = Vec::new();
    for f in files {
        if f.size == 0 || !data::is_mounted_name(f.name()) {
            continue;
        }
        match out.iter_mut().find(|c| c.dir == f.folder()) {
            Some(c) => c.files.push(f.clone()),
            None => out.push(FolderCandidate {
                dir: f.folder().to_string(),
                status: DataStatus::default(),
                files: vec![f.clone()],
            }),
        }
    }
    for c in &mut out {
        c.files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut present: Vec<(String, u64)> = c
            .files
            .iter()
            .map(|f| (f.name().to_string(), f.size))
            .collect();
        present.sort();
        let has = |want: &str| present.iter().any(|(n, _)| n.eq_ignore_ascii_case(want));
        c.status = DataStatus {
            dir: PathBuf::from(&c.dir),
            missing_required: REQUIRED.iter().copied().filter(|r| !has(r)).collect(),
            missing_expected: EXPECTED.iter().copied().filter(|r| !has(r)).collect(),
            present,
        };
    }
    out.sort_by_key(|c| {
        (
            !c.status.complete(),
            std::cmp::Reverse(c.status.present.len()),
            c.dir.matches('/').count() + usize::from(!c.dir.is_empty()),
            c.dir.clone(),
        )
    });
    out
}

/// Opens one listed file, positioned at `offset` — the seam between the copy and the provider.
pub trait Open {
    /// The file `id` from byte `offset`, and where the reader really starts: `offset`, or `0` when
    /// the source cannot seek (a provider that answers with a pipe) — the copy then restarts that
    /// file.
    fn open(&self, id: &str, offset: u64) -> Result<(Box<dyn Read + Send>, u64), String>;
}

/// A file positioned at `offset` for [`Open::open`]: seeked there, or left at `0` when it cannot
/// seek.
pub fn from_offset(mut file: fs::File, offset: u64) -> (Box<dyn Read + Send>, u64) {
    if offset == 0 {
        return (Box::new(file), 0);
    }
    match file.seek(SeekFrom::Start(offset)) {
        Ok(at) if at == offset => (Box::new(file), offset),
        _ => {
            // Not seekable (a pipe that has not been read from yet): it starts at 0 anyway.
            let _ = file.seek(SeekFrom::Start(0));
            (Box::new(file), 0)
        }
    }
}

/// Files in a real folder, by path relative to it — the desktop's stand-in for a provider, and
/// the tests'.
pub struct FolderOpen {
    pub root: PathBuf,
}

impl Open for FolderOpen {
    fn open(&self, id: &str, offset: u64) -> Result<(Box<dyn Read + Send>, u64), String> {
        let path = self.root.join(id);
        let file = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(from_offset(file, offset))
    }
}

/// What a finished [`copy_in`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Copied {
    /// Archives copied (or finished) this time.
    pub copied: usize,
    /// Archives that were already in place at their size.
    pub kept: usize,
    /// Bytes the copied archives hold — what deleting the original would free.
    pub bytes: u64,
}

/// `name` in `dir`, matched case-insensitively (benilla mounts that way, and a phone's shared
/// storage is case-insensitive while a Linux desktop is not).
fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let n = e.file_name();
        n.to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
            .then(|| e.path())
    })
}

/// What [`copy_in`] will do with each of `files` in `dir`: [`Step::Keep`] an archive already there
/// at its listed size, otherwise copy from where a `.part` left off.
pub fn plan(files: &[Listed], dir: &Path) -> Vec<Step> {
    files
        .iter()
        .map(|f| {
            let done = find_ci(dir, f.name()).and_then(|p| fs::metadata(p).ok());
            match done {
                Some(m) if m.is_file() && m.len() == f.size => Step::Keep,
                _ => match fs::metadata(data::part_path(dir, f.name())) {
                    Ok(m) if m.len() <= f.size => Step::Fetch { offset: m.len() },
                    _ => Step::Fetch { offset: 0 },
                },
            }
        })
        .collect()
}

/// The bytes still to copy for `files` into `dir` — what the free-space check needs.
pub fn bytes_to_copy(files: &[Listed], dir: &Path) -> u64 {
    files
        .iter()
        .zip(plan(files, dir))
        .map(|(f, s)| match s {
            Step::Keep => 0,
            Step::Fetch { offset } => f.size - offset,
        })
        .sum()
}

/// Copy `files` (one candidate's archives) into `dir`:
///
/// - an archive already there at its listed size is kept;
/// - a `.<name>.part` ([`crate::data::part_path`]) from an earlier attempt resumes;
/// - the copy goes in [`CHUNK`]s, `keep_going()` checked between them — a stop answers
///   `Err("paused")` and leaves the `.part` for the next attempt;
/// - the size and the MPQ header ([`mpq_looks_whole`]) are checked before the rename to the final
///   name; a file that fails is discarded.
///
/// `report` sees [`Progress`] with the state `copying`, `resuming`, `checking`, `paused` or
/// `done`, as the download's.
pub fn copy_in(
    open: &dyn Open,
    files: &[Listed],
    dir: &Path,
    report: impl FnMut(&Progress),
    keep_going: &dyn Fn() -> bool,
) -> Result<Copied, String> {
    copy_in_chunked(open, files, dir, report, keep_going, CHUNK)
}

fn copy_in_chunked(
    open: &dyn Open,
    files: &[Listed],
    dir: &Path,
    mut report: impl FnMut(&Progress),
    keep_going: &dyn Fn() -> bool,
    chunk: u64,
) -> Result<Copied, String> {
    // Only archives benilla mounts are ever written: the names come from another app.
    if let Some(bad) = files.iter().find(|f| !data::is_mounted_name(f.name())) {
        return Err(format!(
            "refusing to copy {:?}: not a game archive",
            bad.path
        ));
    }
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let steps = plan(files, dir);
    let mut p = Progress {
        file_count: files.len(),
        bytes_total: files.iter().map(|f| f.size).sum(),
        ..Default::default()
    };
    p.bytes_done = files
        .iter()
        .zip(&steps)
        .map(|(f, s)| match s {
            Step::Keep => f.size,
            Step::Fetch { offset } => *offset,
        })
        .sum();
    let mut out = Copied::default();
    let mut buf = vec![0u8; BUF];
    for (i, (file, step)) in files.iter().zip(&steps).enumerate() {
        let Step::Fetch { offset } = *step else {
            out.kept += 1;
            continue;
        };
        let name = file.name();
        let part = data::part_path(dir, name);
        p.file = name.to_string();
        p.file_index = i;
        p.state = if offset > 0 { "resuming" } else { "copying" }.into();
        report(&p);
        if offset < file.size {
            let (mut src, at) = open.open(&file.id, offset)?;
            let mut written = match at {
                a if a == offset => offset,
                0 => {
                    // The source cannot start mid-file: this one starts over.
                    p.bytes_done -= offset;
                    0
                }
                a => {
                    return Err(format!(
                        "{name}: the folder opened it at byte {a}, not {offset}"
                    ))
                }
            };
            let mut dst = if written > 0 {
                fs::OpenOptions::new().append(true).open(&part)
            } else {
                fs::File::create(&part)
            }
            .map_err(|e| format!("{}: {e}", part.display()))?;
            while written < file.size {
                if !keep_going() {
                    p.state = "paused".into();
                    report(&p);
                    return Err("paused".into());
                }
                let end = (written + chunk).min(file.size);
                while written < end {
                    let want =
                        usize::try_from(end - written).map_or(buf.len(), |n| n.min(buf.len()));
                    let n = read_some(&mut src, &mut buf[..want])
                        .map_err(|e| format!("reading {}: {e}", file.path))?;
                    if n == 0 {
                        let _ = dst.sync_data();
                        return Err(format!(
                            "{}: the folder gave {written} of {} bytes — is it still being \
                             copied? Try again to resume.",
                            file.path, file.size
                        ));
                    }
                    dst.write_all(&buf[..n])
                        .map_err(|e| format!("writing {}: {e}", part.display()))?;
                    written += n as u64;
                    p.bytes_done += n as u64;
                }
                report(&p);
            }
            dst.sync_all()
                .map_err(|e| format!("writing {}: {e}", part.display()))?;
        }
        p.state = "checking".into();
        report(&p);
        let len = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if len != file.size || !mpq_looks_whole(&part) {
            let _ = fs::remove_file(&part);
            return Err(format!(
                "{} is not a whole archive (cut short?) and was discarded — copy the folder \
                 again from the original client.",
                file.path
            ));
        }
        // A stale copy under another spelling would be mounted beside the new one.
        if let Some(old) = find_ci(dir, name) {
            if old.file_name().and_then(|n| n.to_str()) != Some(name) {
                let _ = fs::remove_file(old);
            }
        }
        fs::rename(&part, dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        out.copied += 1;
        out.bytes += file.size;
    }
    p.state = "done".into();
    p.file.clear();
    report(&p);
    Ok(out)
}

/// `read`, retried when a signal interrupts it.
fn read_some(r: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match r.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

/// Where an MPQ header may start: on a 512-byte boundary, within the first MiB.
const HEADER_SEARCH: u64 = 1 << 20;

/// Whether the file at `path` looks like a whole MPQ archive: its header (`MPQ\x1A`, at offset 0
/// in every 1.12.1 archive, otherwise on a 512-byte boundary or behind an `MPQ\x1B` user-data
/// block) places the hash table and the block table inside the file. Those tables are written
/// last, at the end, so a copy cut short — wherever it came from: MTP, the Storage Access
/// Framework, Finder — fails this.
pub fn mpq_looks_whole(path: &Path) -> bool {
    fn u32_at(b: &[u8], at: usize) -> u64 {
        u64::from(u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]))
    }
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return false;
    };
    let mut at = 0u64;
    let mut head = [0u8; 32];
    while at < len.min(HEADER_SEARCH) {
        if f.seek(SeekFrom::Start(at)).is_err() || f.read_exact(&mut head).is_err() {
            return false;
        }
        match &head[..4] {
            b"MPQ\x1A" => {
                let (hash_pos, block_pos) = (u32_at(&head, 16), u32_at(&head, 20));
                let (hash_count, block_count) = (u32_at(&head, 24), u32_at(&head, 28));
                let hash_end = at + hash_pos + hash_count * 16;
                let block_end = at + block_pos + block_count * 16;
                return hash_count > 0 && hash_end <= len && block_end <= len;
            }
            // A user-data block: the archive's header is further in.
            b"MPQ\x1B" if u32_at(&head, 8) > 0 => at += u32_at(&head, 8),
            _ => at += 512,
        }
    }
    false
}

/// The archives of `status` that do not look whole ([`mpq_looks_whole`]).
pub fn cut_short(status: &DataStatus) -> Vec<String> {
    status
        .present
        .iter()
        .filter(|(name, _)| !mpq_looks_whole(&status.dir.join(name)))
        .map(|(name, _)| name.clone())
        .collect()
}

/// No archive of `status` (nor its folder) was modified within `quiet`: a copy from a PC into it
/// is not still running. A time in the future counts as recent.
pub fn settled(status: &DataStatus, quiet: Duration) -> bool {
    let now = SystemTime::now();
    let quiet_since = |p: &Path| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= quiet)
    };
    quiet_since(&status.dir)
        && status
            .present
            .iter()
            .all(|(name, _)| quiet_since(&status.dir.join(name)))
}

/// Whether a file moves from the folder `from` into the folder `to` by a rename — the same mount,
/// so the import is instant — probed with a scratch file that is removed either way.
pub fn rename_works(from: &Path, to: &Path) -> bool {
    let probe = format!(".vanillish-probe-{}", std::process::id());
    let (src, dst) = (from.join(&probe), to.join(&probe));
    if fs::write(&src, b"probe").is_err() {
        return false;
    }
    let ok = fs::rename(&src, &dst).is_ok();
    let _ = fs::remove_file(if ok { &dst } else { &src });
    ok
}

/// Why the folder of `status` must not be moved into `to` before the app exists, if it must not
/// (the launch's import, `boot::import_from_documents`, where a copy or a half-written archive
/// must never be taken):
///
/// 1. it changed within `quiet` ([`settled`]: a copy from a PC still running into it — the move
///    would take a half-written archive and pull the folder from under the next file);
/// 2. an archive's MPQ header places its tables past its end ([`cut_short`]);
/// 3. moving it would not be a rename ([`rename_works`]: another mount, so the move would fall
///    back to a copy).
///
/// The rename is probed last: its scratch file comes and goes in the folder, which makes the
/// folder itself "just changed" to [`settled`].
pub fn launch_guard(status: &DataStatus, to: &Path, quiet: Duration) -> Result<(), String> {
    if !settled(status, quiet) {
        return Err("it is still changing (a copy still running?)".into());
    }
    let cut = cut_short(status);
    if !cut.is_empty() {
        return Err(format!("{} look cut short", cut.join(", ")));
    }
    if !rename_works(&status.dir, to) {
        return Err("moving it would be a copy, not a rename".into());
    }
    Ok(())
}

/// The desktop's listing of a real folder, walked by the Java side's rules (`FolderImport.java`):
/// breadth first to `max_depth`, names starting with `.` skipped, [`SKIP_DIRS`] not entered, at
/// most [`MAX_FOLDERS`] folders, only `*.mpq` files listed. Ids are paths relative to `root`.
pub fn list_folder(root: &Path, max_depth: u32) -> Listing {
    let mut listing = Listing {
        label: root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string()),
        ..Default::default()
    };
    let mut queue = std::collections::VecDeque::from([(String::new(), 0u32)]);
    let mut folders = 0;
    while let Some((rel, depth)) = queue.pop_front() {
        folders += 1;
        if folders > MAX_FOLDERS {
            listing.truncated = true;
            break;
        }
        let Ok(rd) = fs::read_dir(root.join(&rel)) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let Ok(kind) = e.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if depth < max_depth && !SKIP_DIRS.contains(&name.to_ascii_lowercase().as_str()) {
                    queue.push_back((path, depth + 1));
                }
            } else if kind.is_file() && name.to_ascii_lowercase().ends_with(".mpq") {
                let meta = e.metadata().ok();
                let modified_ms = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis() as i64);
                listing.files.push(Listed {
                    id: path.clone(),
                    path,
                    size: meta.map_or(0, |m| m.len()),
                    modified_ms,
                });
            }
        }
    }
    listing
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("inilla-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A small archive with a v1 header whose hash and block tables end the file, as a real
    /// archive's do. `seed` varies the payload so files differ.
    fn fake_mpq(size: usize, seed: u8) -> Vec<u8> {
        assert!(size >= 32 + 64);
        let mut b: Vec<u8> = (0..size)
            .map(|i| (i as u8).wrapping_mul(31) ^ seed)
            .collect();
        b[..4].copy_from_slice(b"MPQ\x1A");
        b[4..8].copy_from_slice(&32u32.to_le_bytes());
        b[8..12].copy_from_slice(&(size as u32).to_le_bytes());
        let hash_pos = (size - 64) as u32;
        let block_pos = (size - 32) as u32;
        b[16..20].copy_from_slice(&hash_pos.to_le_bytes());
        b[20..24].copy_from_slice(&block_pos.to_le_bytes());
        b[24..28].copy_from_slice(&2u32.to_le_bytes());
        b[28..32].copy_from_slice(&2u32.to_le_bytes());
        b
    }

    /// A client folder `root/<sub>` holding every required archive; returns the listing.
    fn client(root: &Path, sub: &str, size: usize) -> Listing {
        let data = root.join(sub);
        fs::create_dir_all(&data).unwrap();
        for (i, n) in REQUIRED.iter().enumerate() {
            fs::write(data.join(n), fake_mpq(size + i * 7, i as u8)).unwrap();
        }
        list_folder(root, MAX_DEPTH)
    }

    #[test]
    fn the_java_listing_parses() {
        let json = r#"{"label":"World of Warcraft","truncated":false,
            "files":[{"path":"Data/dbc.MPQ","size":15731508,"modified":1131667200000,
                      "doc":"primary:Download/WoW/World of Warcraft/Data/dbc.MPQ"}]}"#;
        let l = Listing::parse(json).unwrap();
        assert_eq!(l.label, "World of Warcraft");
        assert_eq!(l.files[0].name(), "dbc.MPQ");
        assert_eq!(l.files[0].folder(), "Data");
        assert_eq!(
            l.files[0].id,
            "primary:Download/WoW/World of Warcraft/Data/dbc.MPQ"
        );
        assert_eq!(l.files[0].modified_ms, 1_131_667_200_000);
        // org.json escapes every `/` (and writes what it was given, field order aside).
        let java = r#"{"truncated":true,"files":[{"modified":0,"doc":"primary:Download\/WoW\/Data\/dbc.MPQ","size":7,"path":"Data\/dbc.MPQ"}],"label":"WoW"}"#;
        let l = Listing::parse(java).unwrap();
        assert!(l.truncated);
        assert_eq!(l.files[0].path, "Data/dbc.MPQ");
        assert_eq!(l.files[0].id, "primary:Download/WoW/Data/dbc.MPQ");
        assert!(Listing::parse("not json").is_err());
        assert_eq!(
            parse_pending(r#"{"tree":"content://x/tree/primary%3AWoW","label":"WoW"}"#),
            Some(("content://x/tree/primary%3AWoW".into(), "WoW".into()))
        );
        assert_eq!(parse_pending(r#"{"tree":""}"#), None);
        assert_eq!(parse_pending("null"), None);
    }

    #[test]
    fn candidates_are_folders_complete_first() {
        let listed = |path: &str, size| Listed {
            path: path.into(),
            size,
            modified_ms: 0,
            id: path.into(),
        };
        let mut files: Vec<Listed> = REQUIRED
            .iter()
            .map(|n| listed(&format!("World of Warcraft/Data/{n}"), 10))
            .collect();
        // A backup with more archives but not the whole set, loose ones at the top, an empty one
        // and a file that is not an archive.
        for n in &["dbc.MPQ", "patch.MPQ", "patch-2.MPQ"] {
            files.push(listed(&format!("Old/Data/{n}"), 10));
        }
        files.push(listed("speech.MPQ", 10));
        files.push(listed("World of Warcraft/Data/patch-3.MPQ", 0));
        files.push(listed("World of Warcraft/Data/base.MPQ", 10));
        let c = candidates(&files);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].dir, "World of Warcraft/Data");
        assert!(c[0].status.complete());
        assert_eq!(c[0].files.len(), REQUIRED.len());
        assert_eq!(c[0].status.missing_expected, vec!["patch-2.MPQ"]);
        assert_eq!(c[1].dir, "Old/Data");
        assert_eq!(c[1].status.present.len(), 3);
        assert!(!c[1].status.complete());
        assert_eq!(c[2].dir, "");
        assert_eq!(c[0].total_bytes(), 10 * REQUIRED.len() as u64);
    }

    #[test]
    fn the_desktop_listing_walks_like_the_java_side() {
        let root = tmp("walk");
        let listing = client(&root, "World of Warcraft/Data", 200);
        for skipped in ["World of Warcraft/Interface", ".hidden", "a/b/c/d"] {
            fs::create_dir_all(root.join(skipped)).unwrap();
            fs::write(root.join(skipped).join("dbc.MPQ"), fake_mpq(100, 1)).unwrap();
        }
        fs::write(root.join("World of Warcraft/Data/readme.txt"), "x").unwrap();
        fs::create_dir_all(root.join("a/b/c")).unwrap();
        fs::write(root.join("a/b/c/model.mpq"), fake_mpq(100, 2)).unwrap();
        let listing2 = list_folder(&root, MAX_DEPTH);
        assert!(!listing2.truncated);
        let paths: Vec<&str> = listing2.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"a/b/c/model.mpq"), "{paths:?}");
        assert!(!paths.iter().any(|p| p.contains("Interface")
            || p.contains(".hidden")
            || p.contains("/d/")
            || p.ends_with(".txt")));
        assert_eq!(listing2.files.len(), listing.files.len() + 1);
        assert_eq!(
            listing2.label,
            root.file_name().unwrap().to_string_lossy().as_ref()
        );
    }

    /// A picked folder that is not a client (a whole Download, say) is not walked forever: the
    /// listing stops after [`MAX_FOLDERS`] folders and says so.
    #[test]
    fn a_huge_folder_is_listed_in_part_and_says_so() {
        let root = tmp("huge");
        fs::write(root.join("dbc.MPQ"), fake_mpq(100, 1)).unwrap();
        for i in 0..MAX_FOLDERS {
            fs::create_dir(root.join(format!("f{i:03}"))).unwrap();
        }
        let listing = list_folder(&root, MAX_DEPTH);
        assert!(listing.truncated);
        assert_eq!(listing.files.len(), 1, "the top folder was read first");
        // Exactly the limit (the picked folder and one less below it) is a whole listing.
        fs::remove_dir(root.join("f000")).unwrap();
        assert!(!list_folder(&root, MAX_DEPTH).truncated);
    }

    #[test]
    fn a_folder_is_copied_checked_and_named() {
        let src = tmp("copy-src");
        let listing = client(&src, "World of Warcraft/Data", 3000);
        let found = candidates(&listing.files);
        assert!(found[0].status.complete());
        let dst = tmp("copy-dst");
        let open = FolderOpen { root: src.clone() };
        let mut states = Vec::new();
        let done = copy_in_chunked(
            &open,
            &found[0].files,
            &dst,
            |p| {
                assert!(p.bytes_done <= p.bytes_total);
                states.push(p.state.clone());
            },
            &|| true,
            1024,
        )
        .unwrap();
        assert_eq!(done.copied, REQUIRED.len());
        assert_eq!(done.kept, 0);
        assert_eq!(done.bytes, found[0].total_bytes());
        let st = data::status(&dst);
        assert!(st.complete());
        assert!(cut_short(&st).is_empty());
        for n in REQUIRED {
            assert_eq!(
                fs::read(dst.join(n)).unwrap(),
                fs::read(src.join("World of Warcraft/Data").join(n)).unwrap()
            );
            assert!(!data::part_path(&dst, n).exists());
        }
        assert_eq!(states.last().map(String::as_str), Some("done"));
        assert!(states.iter().any(|s| s == "checking"));
        // The original is still there: a copy, never a move.
        assert_eq!(list_folder(&src, MAX_DEPTH).files.len(), REQUIRED.len());
        // A second run keeps everything.
        let again = copy_in(&open, &found[0].files, &dst, |_| {}, &|| true).unwrap();
        assert_eq!((again.copied, again.kept), (0, REQUIRED.len()));
        assert_eq!(bytes_to_copy(&found[0].files, &dst), 0);
    }

    #[test]
    fn a_paused_copy_resumes_from_its_part() {
        let src = tmp("pause-src");
        let listing = client(&src, "Data", 5000);
        let files = candidates(&listing.files).remove(0).files;
        let dst = tmp("pause-dst");
        let open = FolderOpen { root: src.clone() };
        // Stop after the third chunk: the first archive is part-way.
        let calls = Cell::new(0);
        let keep = || {
            calls.set(calls.get() + 1);
            calls.get() <= 3
        };
        let err = copy_in_chunked(&open, &files, &dst, |_| {}, &keep, 1024).unwrap_err();
        assert_eq!(err, "paused");
        let first = files[0].name();
        let part = data::part_path(&dst, first);
        assert_eq!(fs::metadata(&part).unwrap().len(), 3 * 1024);
        assert!(!dst.join(first).exists());
        assert_eq!(plan(&files, &dst)[0], Step::Fetch { offset: 3 * 1024 });
        let total: u64 = files.iter().map(|f| f.size).sum();
        assert_eq!(bytes_to_copy(&files, &dst), total - 3 * 1024);
        // Resumed: the first progress report already counts the part.
        let mut first_report = None;
        copy_in_chunked(
            &open,
            &files,
            &dst,
            |p| {
                first_report.get_or_insert((p.state.clone(), p.bytes_done));
            },
            &|| true,
            1024,
        )
        .unwrap();
        assert_eq!(first_report, Some(("resuming".to_string(), 3 * 1024)));
        assert!(data::status(&dst).complete());
        assert_eq!(
            fs::read(dst.join(first)).unwrap(),
            fs::read(src.join("Data").join(first)).unwrap()
        );
    }

    /// A provider whose files cannot seek (a pipe): every open starts at 0.
    struct NoSeek(FolderOpen);

    impl Open for NoSeek {
        fn open(&self, id: &str, offset: u64) -> Result<(Box<dyn Read + Send>, u64), String> {
            let (r, _) = self.0.open(id, 0)?;
            let _ = offset;
            Ok((r, 0))
        }
    }

    #[test]
    fn a_source_that_cannot_seek_restarts_the_file() {
        let src = tmp("noseek-src");
        let listing = client(&src, "Data", 2000);
        let files = candidates(&listing.files).remove(0).files;
        let dst = tmp("noseek-dst");
        // A part from an earlier attempt, holding junk.
        let name = files[0].name();
        fs::write(data::part_path(&dst, name), vec![0xEE; 700]).unwrap();
        let open = NoSeek(FolderOpen { root: src.clone() });
        let mut max_done = 0;
        copy_in_chunked(
            &open,
            &files,
            &dst,
            |p| max_done = max_done.max(p.bytes_done),
            &|| true,
            512,
        )
        .unwrap();
        assert_eq!(max_done, files.iter().map(|f| f.size).sum::<u64>());
        assert_eq!(
            fs::read(dst.join(name)).unwrap(),
            fs::read(src.join("Data").join(name)).unwrap()
        );
    }

    #[test]
    fn a_cut_short_source_is_refused() {
        let src = tmp("short-src");
        let listing = client(&src, "Data", 1500);
        let mut files = candidates(&listing.files).remove(0).files;
        let dst = tmp("short-dst");
        let open = FolderOpen { root: src.clone() };
        // The folder gives fewer bytes than it listed: the part stays for a resume.
        files[0].size += 100;
        let err = copy_in_chunked(&open, &files, &dst, |_| {}, &|| true, 1024).unwrap_err();
        assert!(err.contains("still being copied"), "{err}");
        assert!(data::part_path(&dst, files[0].name()).exists());
        // An archive whose tables lie past its end (the source itself cut short) is discarded.
        files[0].size -= 100;
        let name = files[0].name().to_string();
        let whole = fs::read(src.join("Data").join(&name)).unwrap();
        fs::write(src.join("Data").join(&name), &whole[..whole.len() - 40]).unwrap();
        let listing = list_folder(&src, MAX_DEPTH);
        let files = candidates(&listing.files).remove(0).files;
        let _ = fs::remove_file(data::part_path(&dst, &name));
        let err = copy_in_chunked(&open, &files, &dst, |_| {}, &|| true, 1024).unwrap_err();
        assert!(err.contains("not a whole archive"), "{err}");
        assert!(!data::part_path(&dst, &name).exists());
        assert!(!dst.join(&name).exists());
    }

    #[test]
    fn names_that_are_not_archives_are_refused() {
        let dst = tmp("names-dst");
        let open = FolderOpen { root: dst.clone() };
        for path in [
            "Data/../evil.MPQ",
            "Data/realmlist.wtf",
            "Data/.dbc.MPQ.part",
        ] {
            let f = Listed {
                path: path.into(),
                size: 1,
                modified_ms: 0,
                id: path.into(),
            };
            let err = copy_in(&open, &[f], &dst, |_| {}, &|| true).unwrap_err();
            assert!(err.contains("not a game archive"), "{err}");
        }
    }

    #[test]
    fn a_stale_archive_under_another_spelling_is_replaced() {
        let src = tmp("case-src");
        let listing = client(&src, "Data", 800);
        let files = candidates(&listing.files).remove(0).files;
        let dst = tmp("case-dst");
        // `DBC.MPQ` at the wrong size: it is not kept, and does not survive beside `dbc.MPQ`.
        fs::write(dst.join("DBC.MPQ"), b"stale").unwrap();
        let open = FolderOpen { root: src };
        copy_in(&open, &files, &dst, |_| {}, &|| true).unwrap();
        let st = data::status(&dst);
        assert_eq!(st.present.len(), REQUIRED.len(), "{:?}", st.present);
    }

    #[test]
    fn mpq_headers_say_whether_a_file_is_whole() {
        let d = tmp("mpq");
        let whole = fake_mpq(400, 3);
        fs::write(d.join("whole.MPQ"), &whole).unwrap();
        assert!(mpq_looks_whole(&d.join("whole.MPQ")));
        fs::write(d.join("short.MPQ"), &whole[..300]).unwrap();
        assert!(!mpq_looks_whole(&d.join("short.MPQ")));
        fs::write(d.join("junk.MPQ"), vec![0u8; 400]).unwrap();
        assert!(!mpq_looks_whole(&d.join("junk.MPQ")));
        fs::write(d.join("tiny.MPQ"), b"MPQ\x1A").unwrap();
        assert!(!mpq_looks_whole(&d.join("tiny.MPQ")));
        assert!(!mpq_looks_whole(&d.join("missing.MPQ")));
        // Behind a 512-byte user-data block, the tables are relative to the real header.
        let mut user = vec![0u8; 512];
        user[..4].copy_from_slice(b"MPQ\x1B");
        user[8..12].copy_from_slice(&512u32.to_le_bytes());
        user.extend_from_slice(&whole);
        fs::write(d.join("user.MPQ"), &user).unwrap();
        assert!(mpq_looks_whole(&d.join("user.MPQ")));
        fs::write(d.join("user-short.MPQ"), &user[..800]).unwrap();
        assert!(!mpq_looks_whole(&d.join("user-short.MPQ")));
    }

    #[test]
    fn a_folder_still_being_written_is_not_settled() {
        let root = tmp("settled");
        client(&root, "Data", 200);
        let st = data::status(&root.join("Data"));
        assert!(!settled(&st, Duration::from_secs(15)));
        assert!(settled(&st, Duration::ZERO));
        let old = SystemTime::now() - Duration::from_secs(3600);
        for (n, _) in &st.present {
            let f = fs::File::options()
                .append(true)
                .open(st.dir.join(n))
                .unwrap();
            f.set_modified(old).unwrap();
        }
        // The folder itself was just written (a new file in it).
        assert!(!settled(&st, Duration::from_secs(15)));
        fs::File::open(&st.dir).unwrap().set_modified(old).unwrap();
        assert!(settled(&st, Duration::from_secs(15)));
        // One archive touched again.
        let f = fs::File::options()
            .append(true)
            .open(st.dir.join(&st.present[0].0))
            .unwrap();
        f.set_modified(SystemTime::now()).unwrap();
        assert!(!settled(&st, Duration::from_secs(15)));
    }

    #[test]
    fn a_rename_is_probed_and_cleaned_up() {
        let a = tmp("probe-a");
        let b = tmp("probe-b");
        assert!(rename_works(&a, &b));
        assert!(!rename_works(&a, &b.join("missing")));
        assert!(!rename_works(&a.join("missing"), &b));
        assert_eq!(fs::read_dir(&a).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&b).unwrap().count(), 0);
    }

    /// The launch's guard takes a folder left alone and whole — and its rename probe, which
    /// touches the folder, must not be what makes it look "still changing".
    #[test]
    fn the_launch_guard_takes_a_settled_whole_folder() {
        let root = tmp("guard");
        client(&root, "Data", 300);
        let to = tmp("guard-to");
        let st = data::status(&root.join("Data"));
        let quiet = Duration::from_secs(15);
        let age = |p: &Path| {
            let old = SystemTime::now() - Duration::from_secs(3600);
            fs::File::open(p).unwrap().set_modified(old).unwrap();
        };
        let age_all = || {
            for (n, _) in &st.present {
                age(&st.dir.join(n));
            }
            age(&st.dir);
        };
        let err = launch_guard(&st, &to, quiet).unwrap_err();
        assert!(err.contains("still changing"), "{err}");
        age_all();
        assert_eq!(launch_guard(&st, &to, quiet), Ok(()));
        assert_eq!(fs::read_dir(&to).unwrap().count(), 0);
        assert_eq!(data::status(&st.dir).present, st.present);
        // Moving into a folder elsewhere (here: one that is not there) is not a rename.
        age_all();
        let err = launch_guard(&st, &to.join("missing"), quiet).unwrap_err();
        assert!(err.contains("not a rename"), "{err}");
        // An archive cut short, however long ago.
        let first = st.dir.join(&st.present[0].0);
        let whole = fs::read(&first).unwrap();
        fs::write(&first, &whole[..whole.len() - 40]).unwrap();
        age_all();
        let err = launch_guard(&st, &to, quiet).unwrap_err();
        assert!(
            err.contains(&st.present[0].0) && err.contains("cut short"),
            "{err}"
        );
    }

    #[test]
    fn a_file_opens_at_its_offset() {
        let d = tmp("offset");
        fs::write(d.join("f"), b"0123456789").unwrap();
        let (mut r, at) = from_offset(fs::File::open(d.join("f")).unwrap(), 4);
        assert_eq!(at, 4);
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        assert_eq!(s, "456789");
    }
}
