//! **Getting the game data onto the device**, the app half: the status the setup screen shows,
//! and the background jobs it starts — scan-and-import from Documents, fetch a server's manifest,
//! download (resumable, verified per file). The logic is `inilla_core::data`; this is threads,
//! progress and the HTTP client.
//!
//! benilla opens the archives when the app is built, so data that arrives while it runs needs one
//! relaunch; the setup screen says so and offers it. (A Documents import found at launch needs
//! none — `boot::prepare` moves it into place before the app exists.)
//!
//! Vanillish: a third way in, the **folder import** — a folder the player picks with the system's
//! picker (Android's Storage Access Framework), copied in by `inilla_core::import` on the same
//! job thread, with the same progress, Pause and resume as the download.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use bevy::prelude::*;

use inilla_core::addons::{self as ka, AddonManifest, HostLocales, SyncReport};
use inilla_core::data::{self as kd, DataStatus, Fetch, Manifest, Progress};
use inilla_core::import as ki;
use inilla_platform::picker::{self, Picked};

use super::ui::words;

/// Vanillish: how long a `Data` folder must have been left alone before it is taken — a copy from
/// a PC still running into it must not be moved from under it.
pub(crate) const SETTLE: Duration = Duration::from_secs(15);

/// Room left beyond the archives themselves when a copy or download starts.
const SPARE: u64 = 200_000_000;

/// What a finished job produced.
enum Outcome {
    Manifest(Box<Offer>),
    Downloaded(Option<SyncReport>),
    Addons(SyncReport),
    Imported(usize),
    NothingToImport,
    /// Vanillish: a folder import finished: what it did, what the original holds (what deleting
    /// it would free), and what it lacked.
    Copied {
        label: String,
        done: ki::Copied,
        original_bytes: u64,
        missing: Vec<&'static str>,
    },
    /// Vanillish: the picked folder holds no archive.
    NothingInFolder {
        label: String,
        truncated: bool,
    },
}

/// What a host offers for one language: the archives, the addons, and where they come from.
pub(crate) struct Offer {
    pub(crate) manifest: Manifest,
    pub(crate) addons: AddonManifest,
    /// The host's languages, when it has more than one.
    pub(crate) locales: Option<HostLocales>,
    /// The language this offer is for (`enUS` when the host does not say).
    pub(crate) locale: String,
    /// The base URL the archives and addons of that language come from.
    pub(crate) base: String,
}

/// benilla's addon folder, `<benilla-config>/AddOns`.
pub(crate) fn addons_dir() -> Option<PathBuf> {
    crate::local_state::home().map(|h| h.join("AddOns"))
}

struct Job {
    label: &'static str,
    progress: Arc<Mutex<Progress>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<Outcome, String>>>,
}

#[derive(Resource)]
pub(crate) struct DataState {
    /// Where the data lives (the iOS container's, or `$WOW_DATA` on desktop).
    pub(crate) dir: Option<PathBuf>,
    pub(crate) status: DataStatus,
    /// The archives were complete when the app was built (so benilla has them open).
    pub(crate) ready_at_boot: bool,
    pub(crate) manifest: Option<Manifest>,
    /// The checked host's addons and languages, and the base URL the manifest came from.
    pub(crate) addons: Option<AddonManifest>,
    pub(crate) locales: Option<HostLocales>,
    pub(crate) locale: Option<String>,
    pub(crate) base: Option<String>,
    /// The last job's result, for the setup screen.
    pub(crate) message: Option<(bool, String)>,
    /// Vanillish: a picked folder whose copy has not finished, `(tree, label)` — asked of the
    /// platform at start and after each folder import, not every frame.
    pub(crate) pending_folder: Option<(String, String)>,
    /// Vanillish: the archives as the app was built (`status.present` then), and whether a job
    /// has put one in place since — what [`Self::needs_restart`] compares against.
    boot_present: Vec<(String, u64)>,
    replaced: bool,
    job: Option<Job>,
}

impl DataState {
    pub(crate) fn busy(&self) -> Option<(&'static str, Progress)> {
        self.job.as_ref().map(|j| {
            (
                j.label,
                j.progress.lock().map(|p| p.clone()).unwrap_or_default(),
            )
        })
    }

    /// Data is complete now but benilla does not have it as it is: a relaunch is needed. It did
    /// not have it at boot, or (Vanillish) an archive was added, removed or replaced since:
    /// benilla holds every archive's tables from boot and reopens the files by path, so reads
    /// from a replaced one would take the old offsets into the new file.
    pub(crate) fn needs_restart(&self) -> bool {
        self.status.complete()
            && (!self.ready_at_boot || self.replaced || self.status.present != self.boot_present)
    }

    pub(crate) fn refresh(&mut self) {
        if let Some(d) = &self.dir {
            self.status = kd::status(d);
        }
    }

    fn start(
        &mut self,
        label: &'static str,
        f: impl FnOnce(Arc<Mutex<Progress>>, Arc<AtomicBool>) -> Result<Outcome, String>
            + Send
            + 'static,
    ) {
        if self.job.is_some() {
            return;
        }
        // Vanillish: without shared storage the folders are a private stand-in, which the next
        // launch does not use: nothing is written there (`paths::storage_unavailable`).
        if label != "manifest" && inilla_platform::paths::storage_unavailable() {
            self.message = Some((false, words::STORAGE_UNAVAILABLE.into()));
            return;
        }
        let progress = Arc::new(Mutex::new(Progress::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (progress.clone(), stop.clone());
        let handle = std::thread::Builder::new()
            .name(format!("inilla-{label}"))
            .spawn(move || f(p, s))
            .ok();
        self.message = None;
        self.job = Some(Job {
            label,
            progress,
            stop,
            handle,
        });
    }

    /// Look through Documents for a `Data` folder and move it into place.
    pub(crate) fn scan_import(&mut self) {
        let (Some(docs), Some(dir)) = (inilla_platform::paths::documents(), self.dir.clone())
        else {
            self.message = Some((
                false,
                "There is no Documents folder to import from on this platform.".into(),
            ));
            return;
        };
        self.start("import", move |progress, _| {
            let found = kd::find_imports(&docs);
            let Some(c) = found.into_iter().max_by_key(|c| c.status.present.len()) else {
                return Ok(Outcome::NothingToImport);
            };
            // Vanillish: not a folder a PC is still copying into (the boot import's guard too).
            if !cfg!(target_os = "ios") {
                let shown = c.dir.strip_prefix(&docs).unwrap_or(&c.dir).display();
                if !ki::settled(&c.status, SETTLE) {
                    return Err(format!(
                        "{shown} changed in the last few seconds — a copy may still be running. \
                         Let it finish, then check again."
                    ));
                }
                let cut = ki::cut_short(&c.status);
                if !cut.is_empty() {
                    return Err(format!(
                        "{} in {shown} look cut short — copy them in again.",
                        cut.join(", ")
                    ));
                }
            }
            let n = kd::import(&c.dir, &dir, |name, i, total| {
                if let Ok(mut p) = progress.lock() {
                    p.file = name.to_string();
                    p.file_index = i;
                    p.file_count = total;
                    p.state = "moving".into();
                }
            })
            .map_err(|e| format!("import failed: {e}"))?;
            let _ = std::fs::remove_dir(&c.dir);
            Ok(Outcome::Imported(n))
        });
    }

    /// Ask the host at `url` what it offers: its languages, and for `locale` (or its default)
    /// the manifest and the addons.
    pub(crate) fn fetch_manifest(
        &mut self,
        url: String,
        token: Option<String>,
        locale: Option<String>,
    ) {
        self.start("manifest", move |_, _| {
            let http = Http::new(token);
            let locales = ka::fetch_locales(&http, &url);
            let base = match (&locales, &locale) {
                (Some(l), Some(want)) => l.base_for(&url, want),
                _ => url.clone(),
            };
            let manifest = kd::fetch_manifest(&http, &base)?;
            let addons = ka::fetch_addons(&http, &base);
            let locale = manifest
                .locale
                .clone()
                .or(locale)
                .unwrap_or_else(|| "enUS".into());
            Ok(Outcome::Manifest(Box::new(Offer {
                manifest,
                addons,
                locales: locales.filter(|l| l.locales.len() > 1),
                locale,
                base,
            })))
        });
    }

    /// Download what the manifest lists and the device lacks, then the host's addons.
    pub(crate) fn download(&mut self, token: Option<String>) {
        let (Some(manifest), Some(dir), Some(base)) =
            (self.manifest.clone(), self.dir.clone(), self.base.clone())
        else {
            return;
        };
        let addons = self.addons.clone().filter(|a| !a.addons.is_empty());
        self.start("download", move |progress, stop| {
            let http = Http::new(token);
            let keep = move || !stop.load(Ordering::Relaxed);
            let report = |p: &Progress| {
                if let Ok(mut g) = progress.lock() {
                    *g = p.clone();
                }
            };
            kd::download(&http, &base, &manifest, &dir, report, &keep)?;
            let synced = match (addons, addons_dir()) {
                (Some(a), Some(to)) => Some(ka::sync_addons(&http, &base, &a, &to, report, &keep)?),
                _ => None,
            };
            Ok(Outcome::Downloaded(synced))
        });
    }

    /// Install or update the checked host's addons.
    pub(crate) fn sync_addons(&mut self, token: Option<String>) {
        let (Some(addons), Some(base), Some(to)) =
            (self.addons.clone(), self.base.clone(), addons_dir())
        else {
            return;
        };
        self.start("addons", move |progress, stop| {
            let http = Http::new(token);
            let keep = move || !stop.load(Ordering::Relaxed);
            ka::sync_addons(
                &http,
                &base,
                &addons,
                &to,
                |p| {
                    if let Ok(mut g) = progress.lock() {
                        *g = p.clone();
                    }
                },
                &keep,
            )
            .map(Outcome::Addons)
        });
    }

    /// Vanillish: copy the archives in from a folder the player picked (`tree`, as the picker
    /// answered; `label` its name): list it, take the best candidate folder in it, check the free
    /// space, copy with resume (`inilla_core::import::copy_in`). A copy — the original stays — and
    /// the picker's grant is given back once it is done.
    pub(crate) fn import_folder(&mut self, tree: String, label: String) {
        let Some(dir) = self.dir.clone() else {
            self.message = Some((
                false,
                "There is no game data folder on this platform.".into(),
            ));
            return;
        };
        self.start("folder import", move |progress, stop| {
            if let Ok(mut p) = progress.lock() {
                p.state = "looking in".into();
                p.file = label.clone();
            }
            let listing = picker::list(&tree, ki::MAX_DEPTH)?;
            let Some(c) = ki::candidates(&listing.files).into_iter().next() else {
                picker::forget(&tree);
                return Ok(Outcome::NothingInFolder {
                    label,
                    truncated: listing.truncated,
                });
            };
            // Not a folder something still writes into (a copy, a torrent client filling in a
            // file it sized first): the header check cannot tell such a file from a whole one.
            // The grant stays, so Resume tries again.
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            if !ki::listing_settled(&c.files, now_ms, SETTLE) {
                return Err(format!(
                    "{label} changed in the last few seconds — a copy or download may still be \
                     running. Let it finish, then try again."
                ));
            }
            let open = PickedOpen { tree: tree.clone() };
            let need = ki::bytes_to_copy(&c.files, &dir, &ki::Open::origin(&open));
            if let Some(free) = free_space(&dir) {
                if free < need + SPARE {
                    return Err(format!(
                        "Not enough space: {:.1} GB needed, {:.1} GB free.",
                        (need + SPARE) as f64 / 1e9,
                        free as f64 / 1e9
                    ));
                }
            }
            let keep = move || !stop.load(Ordering::Relaxed);
            let report = |p: &Progress| {
                if let Ok(mut g) = progress.lock() {
                    *g = p.clone();
                }
            };
            let done = ki::copy_in(&open, &c.files, &dir, report, &keep)?;
            picker::forget(&tree);
            Ok(Outcome::Copied {
                label,
                done,
                original_bytes: c.total_bytes(),
                missing: c.status.missing_required,
            })
        });
    }

    /// Stop the running download at the next buffer (its `.part` stays for a resume).
    pub(crate) fn pause(&mut self) {
        if let Some(j) = &self.job {
            j.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Bytes free on the data volume, when the platform says.
    pub(crate) fn free_bytes(&self) -> Option<u64> {
        let dir = self.dir.as_ref()?;
        free_space(dir)
    }
}

/// Vanillish: a picked folder's files, through the platform's picker (a provider's file
/// descriptor on Android, a path on the desktop).
struct PickedOpen {
    tree: String,
}

impl ki::Open for PickedOpen {
    fn open(&self, id: &str, offset: u64) -> Result<(Box<dyn Read + Send>, u64), String> {
        let file = picker::open(&self.tree, id)?;
        Ok(ki::from_offset(file, offset))
    }

    fn origin(&self) -> String {
        self.tree.clone()
    }
}

#[cfg(unix)]
fn free_space(dir: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid NUL-terminated path and an out-struct of the right type.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    // `f_bavail` is 32-bit on Darwin and 64-bit on Linux: the casts are only identity on one.
    #[allow(clippy::unnecessary_cast)]
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

#[cfg(not(unix))]
fn free_space(_dir: &std::path::Path) -> Option<u64> {
    None
}

/// The HTTP half of the download, over `ureq` (rustls).
struct Http {
    agent: ureq::Agent,
    token: Option<String>,
}

impl Http {
    fn new(token: Option<String>) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(15))
                .timeout_read(Duration::from_secs(60))
                .build(),
            token: token.filter(|t| !t.trim().is_empty()),
        }
    }

    fn get(&self, url: &str) -> ureq::Request {
        let r = self.agent.get(url).set("User-Agent", words::APP);
        match &self.token {
            Some(t) => r.set("Authorization", &format!("Bearer {}", t.trim())),
            None => r,
        }
    }
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401 | 403, _) => "the server refused the token (401/403)".into(),
        ureq::Error::Status(404, _) => "not found (404) — check the URL".into(),
        ureq::Error::Status(code, _) => format!("the server answered {code}"),
        ureq::Error::Transport(t) => format!("could not reach the server: {t}"),
    }
}

impl Fetch for Http {
    fn text(&self, url: &str) -> Result<String, String> {
        self.get(url)
            .call()
            .map_err(describe)?
            .into_string()
            .map_err(|e| e.to_string())
    }

    fn body(&self, url: &str, offset: u64) -> Result<(Box<dyn Read + Send>, bool), String> {
        let mut req = self.get(url);
        if offset > 0 {
            req = req.set("Range", &format!("bytes={offset}-"));
        }
        let resp = req.call().map_err(describe)?;
        let ranged = resp.status() == 206;
        Ok((Box::new(resp.into_reader()), ranged))
    }
}

pub(crate) struct DataPlugin;

impl Plugin for DataPlugin {
    fn build(&self, app: &mut App) {
        let dir = inilla_platform::paths::data_dir()
            .or_else(|| {
                std::env::var_os("WOW_DATA")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
            })
            .or_else(benilla_formats::wow_data);
        let status = dir.as_deref().map(kd::status).unwrap_or_default();
        // Complete at boot iff benilla was handed it: `boot::prepare` sets `WOW_DATA` only then.
        let ready = status.complete() && std::env::var_os("WOW_DATA").is_none_or(|v| !v.is_empty());
        app.insert_resource(DataState {
            dir,
            boot_present: status.present.clone(),
            status,
            ready_at_boot: ready,
            manifest: None,
            addons: None,
            locales: None,
            locale: None,
            base: None,
            message: None,
            pending_folder: picker::pending(),
            replaced: false,
            job: None,
        })
        .add_systems(Update, (poll_job, poll_picker));
    }
}

/// Vanillish: the folder picker's answer starts the copy. Only while no job runs — the answer
/// waits in the picker until then (it may even arrive before the app is built, when Android
/// brought the app back for it).
fn poll_picker(mut state: ResMut<DataState>) {
    if state.job.is_some() {
        return;
    }
    match picker::poll() {
        Some(Picked::Folder { tree, label }) => {
            info!("inilla: importing from the picked folder {label}");
            state.import_folder(tree, label);
        }
        Some(Picked::Failed(why)) => {
            state.message = Some((false, format!("The folder picker failed: {why}")));
        }
        Some(Picked::Cancelled) | None => {}
    }
}

/// Vanillish: a finished job put archives in place: imported or copied some. A download, or a job
/// that stopped part-way (earlier archives may be in by then), shows in the archives' list itself
/// ([`DataState::needs_restart`] compares it with the boot's); a copy also replaces an archive of
/// the same size when its source is newer, which the list cannot show.
fn puts_archives_in_place(result: &Result<Outcome, String>) -> bool {
    match result {
        Ok(Outcome::Imported(n)) => *n > 0,
        Ok(Outcome::Copied { done, .. }) => done.copied > 0,
        _ => false,
    }
}

fn poll_job(mut state: ResMut<DataState>) {
    let done = state
        .job
        .as_ref()
        .is_some_and(|j| j.handle.as_ref().is_none_or(|h| h.is_finished()));
    if !done {
        return;
    }
    let Some(mut job) = state.job.take() else {
        return;
    };
    let result = job
        .handle
        .take()
        .map(|h| h.join().unwrap_or_else(|_| Err("the job crashed".into())))
        .unwrap_or_else(|| Err("the job could not start".into()));
    state.refresh();
    if job.label == "folder import" {
        state.pending_folder = picker::pending();
    }
    if puts_archives_in_place(&result) {
        state.replaced = true;
    }
    state.message = Some(match result {
        Ok(Outcome::Manifest(offer)) => {
            let offer = *offer;
            let m = &offer.manifest;
            let gb = m.total_bytes() as f64 / 1e9;
            let addons = if offer.addons.addons.is_empty() {
                String::new()
            } else {
                format!(" and {}", addon_names(&offer.addons))
            };
            let text = format!(
                "The server offers {} archives ({gb:.1} GB, {}){addons}.",
                m.files.len(),
                ka::locale_name(&offer.locale),
            );
            state.manifest = Some(offer.manifest);
            state.addons = Some(offer.addons);
            state.locales = offer.locales;
            state.locale = Some(offer.locale);
            state.base = Some(offer.base);
            (true, text)
        }
        Ok(Outcome::Downloaded(synced)) => (
            true,
            match synced {
                Some(r) => format!("Download complete and verified. {}", sync_text(&r)),
                None => "Download complete and verified.".into(),
            },
        ),
        Ok(Outcome::Addons(r)) => (true, sync_text(&r)),
        Ok(Outcome::Imported(n)) => (true, format!("Imported {n} archives.")),
        Ok(Outcome::NothingToImport) => (false, words::nothing_to_import()),
        Ok(Outcome::Copied {
            label,
            done,
            original_bytes,
            missing,
        }) => {
            let n = done.copied + done.kept;
            let mut text = if done.copied == 0 {
                format!("The {n} archives from {label} were already here.")
            } else {
                format!(
                    "Copied {n} archives from {label}. The original folder is still there; \
                     delete it to free {:.1} GB.",
                    original_bytes as f64 / 1e9
                )
            };
            if !missing.is_empty() {
                text.push_str(&format!(
                    " It does not have the whole set: missing {}.",
                    missing.join(", ")
                ));
            }
            (missing.is_empty(), text)
        }
        Ok(Outcome::NothingInFolder { label, truncated }) => (
            false,
            format!(
                "No .MPQ archives in {label}. Pick the Data folder or the World of Warcraft \
                 folder.{}",
                if truncated {
                    " (It is a big folder and only part of it was searched.)"
                } else {
                    ""
                }
            ),
        ),
        Err(e) if e == "paused" => (true, "Paused — Resume continues where it stopped.".into()),
        Err(e) => (false, e),
    });
    info!(
        "inilla: data job {} finished — {}",
        job.label,
        state.message.as_ref().map_or("", |m| m.1.as_str())
    );
}

/// `pfQuest 7.0.1, Bagnon`
pub(crate) fn addon_names(m: &AddonManifest) -> String {
    m.addons
        .iter()
        .map(|a| match &a.version {
            Some(v) if !v.is_empty() => format!("{} {v}", a.name),
            _ => a.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn sync_text(r: &SyncReport) -> String {
    let mut parts = Vec::new();
    if !r.updated.is_empty() {
        let names: Vec<String> = r
            .updated
            .iter()
            .map(|(n, v)| {
                if v.is_empty() {
                    n.clone()
                } else {
                    format!("{n} {v}")
                }
            })
            .collect();
        parts.push(format!("Installed {}.", names.join(", ")));
    } else if r.current > 0 {
        parts.push("Addons are up to date.".to_string());
    }
    if !r.removed.is_empty() {
        parts.push(format!("Removed {}.", r.removed.join(", ")));
    }
    if parts.is_empty() {
        "This server offers no addons.".to_string()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(ready_at_boot: bool, present: Vec<(String, u64)>) -> DataState {
        DataState {
            dir: None,
            boot_present: present.clone(),
            status: DataStatus {
                present,
                ..Default::default()
            },
            ready_at_boot,
            manifest: None,
            addons: None,
            locales: None,
            locale: None,
            base: None,
            message: None,
            pending_folder: None,
            replaced: false,
            job: None,
        }
    }

    /// Vanillish: benilla holds the boot's archive tables, so data that was complete at boot
    /// needs a relaunch once an archive is added, removed or replaced.
    #[test]
    fn an_archive_changed_since_boot_needs_a_relaunch() {
        let whole: Vec<(String, u64)> = kd::REQUIRED.iter().map(|n| (n.to_string(), 10)).collect();
        let mut s = state(true, whole.clone());
        assert!(!s.needs_restart());
        // Replaced at another size, or one more.
        s.status.present[0].1 = 11;
        assert!(s.needs_restart());
        s.status.present = whole.clone();
        s.status.present.push(("patch-3.MPQ".into(), 5));
        assert!(s.needs_restart());
        // Replaced at the same size (a copy of a newer source): the job says so.
        s.status.present = whole.clone();
        assert!(!s.needs_restart());
        let copied = |n| {
            Ok(Outcome::Copied {
                label: "WoW".into(),
                done: ki::Copied {
                    copied: n,
                    ..Default::default()
                },
                original_bytes: 0,
                missing: Vec::new(),
            })
        };
        assert!(!puts_archives_in_place(&copied(0)));
        assert!(puts_archives_in_place(&copied(1)));
        assert!(puts_archives_in_place(&Ok(Outcome::Imported(2))));
        assert!(!puts_archives_in_place(&Err("paused".into())));
        s.replaced = true;
        assert!(s.needs_restart());
        // Not complete at boot: complete now needs one, as before; incomplete never.
        let mut s = state(false, whole.clone());
        assert!(s.needs_restart());
        s.status.present.clear();
        assert!(!s.needs_restart());
    }
}
