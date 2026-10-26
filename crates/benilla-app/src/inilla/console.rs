//! **The remote console** — the fast debug loop without a Mac. With it switched on (Settings →
//! Debug) the app serves a small web page on the local network; open it in any browser on the
//! same Wi-Fi and you get:
//!
//! - the **live session log** (everything the client prints, the log ring's lines),
//! - a **Lua prompt** into the running interface (`/api/lua`, values rendered back as JSON),
//! - **game commands** by binding name, chat lines and slash commands, `ReloadUI()`,
//! - a **screenshot** button, and
//! - the **`benilla-config` files** — crash reports, session logs, screenshots, `config.toml`,
//!   saved variables — to download, and upload (an edited config, an addon's files).
//!
//! Every request but the page itself needs the token shown in Settings (`Authorization: Bearer`,
//! or `?token=`). The server threads never touch the game: they hand commands to the main thread
//! over a channel ([`service`]) and read a state snapshot it publishes once a second.

use std::net::TcpListener;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};

use benilla_ui::script::plain::PlainValue;
use inilla_core::http::{self, Request};
use inilla_core::pad::PadBinding;

use super::synth::{LuaJob, Synth};
use super::Inilla;
use crate::char_select::ClientState;

enum ConsoleCmd {
    Lua {
        chunk: String,
        reply: Sender<Result<Vec<PlainValue>, String>>,
    },
    Press(String),
    Chat(String),
}

struct Running {
    port: u16,
    token: String,
    stop: Arc<AtomicBool>,
}

#[derive(Resource)]
struct Console {
    tx: Sender<ConsoleCmd>,
    rx: Receiver<ConsoleCmd>,
    state: Arc<Mutex<String>>,
    running: Option<Running>,
}

impl Default for Console {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            tx,
            rx,
            state: Arc::new(Mutex::new("{}".into())),
            running: None,
        }
    }
}

pub(crate) struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Console>()
            .add_systems(Update, (manage, service, publish_state));
    }
}

/// A fresh random token (hex, 24 chars). `RandomState` is seeded from the OS per instance.
pub(crate) fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        out.push_str(&format!("{:012x}", h.finish() & 0xffff_ffff_ffff));
    }
    out
}

/// How good an interface's address is for reaching the console from another machine, best first;
/// `None` drops it.
///
/// - `0` Wi-Fi: `en0` on iOS, `wlan0` on Android;
/// - `1` the phone's own network for others: its hotspot (`swlan0`, `ap0`), USB tethering
///   (`rndis0`, `ncm0`);
/// - `2` anything else, in the system's order: a desktop's `eth0`, `enp*`, `wlp*`; Android's USB
///   Ethernet under DeX, Wi-Fi Direct, Bluetooth tethering;
/// - `3` a VPN (`tun0`, `wg0`): last, as only the VPN's other ends can reach it;
/// - dropped: Android's cellular data (`rmnet*`, `ccmni*` — the carrier's NAT, never reachable),
///   the 464XLAT stand-in of an IPv6-only network (`v4-*`, 192.0.0.4, reachable from nowhere) and
///   dummy interfaces.
///
/// Vanillish: iOS only ever needed `en0` first; Android names its interfaces differently. The
/// sort is stable, so within a rank the system's order holds, as it did for all but `en0`. On
/// Windows [`adapter_rank`] ranks instead.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn iface_rank(name: &str) -> Option<u8> {
    let starts = |prefixes: &[&str]| prefixes.iter().any(|p| name.starts_with(p));
    if starts(&["rmnet", "ccmni", "v4-", "dummy"]) {
        None
    } else if name == "en0" || name.starts_with("wlan") {
        Some(0)
    } else if starts(&["swlan", "ap", "rndis", "ncm"]) {
        Some(1)
    } else if starts(&["tun", "wg"]) {
        Some(3)
    } else {
        Some(2)
    }
}

/// Vanillish: [`iface_rank`] for Windows, which names an adapter for the player ("Wi-Fi",
/// "Ethernet 2", "vEthernet (WSL)"): `0` Wi-Fi, `1` Ethernet, `2` anything else, `3` a virtual
/// switch, a VM's or a container's network, a VPN or the loopback — only this PC, or the VPN's
/// other ends, can reach those.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn adapter_rank(name: &str) -> u8 {
    let n = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    // "TAP" alone, as TAP-Windows names its adapters, and not inside another word.
    let tap = n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == "tap");
    if tap
        || has(&[
            "vethernet",
            "wsl",
            "hyper-v",
            "docker",
            "virtualbox",
            "vmware",
            "tailscale",
            "zerotier",
            "wireguard",
            "openvpn",
            "vpn",
            "loopback",
        ])
    {
        3
    } else if has(&["wi-fi", "wifi", "wlan", "wireless"]) {
        0
    } else if n.contains("ethernet") {
        1
    } else {
        2
    }
}

/// This device's IPv4 addresses on its network interfaces (Wi-Fi first, see [`iface_rank`]),
/// loopback excluded.
pub(crate) fn local_addresses() -> Vec<String> {
    // Vanillish: Windows lists its adapters through `inilla_platform::net`, ranked by name.
    #[cfg(windows)]
    let (out, rank) = (inilla_platform::net::ipv4_interfaces(), |n: &str| {
        Some(adapter_rank(n))
    });
    #[cfg(not(windows))]
    let (mut out, rank) = (Vec::<(String, String)>::new(), iface_rank);
    #[cfg(unix)]
    // SAFETY: getifaddrs/freeifaddrs as documented; every pointer is checked before use.
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) == 0 {
            let mut p = ifap;
            while !p.is_null() {
                let ifa = &*p;
                if !ifa.ifa_addr.is_null() && i32::from((*ifa.ifa_addr).sa_family) == libc::AF_INET
                {
                    let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                    if !ip.is_loopback() {
                        let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                            .to_string_lossy()
                            .into_owned();
                        out.push((name, ip.to_string()));
                    }
                }
                p = ifa.ifa_next;
            }
            libc::freeifaddrs(ifap);
        }
    }
    // Wi-Fi first, then the rest; cellular dropped.
    let mut ranked: Vec<(u8, String)> = out
        .into_iter()
        .filter_map(|(n, ip)| rank(&n).map(|r| (r, ip)))
        .collect();
    ranked.sort_by_key(|(r, _)| *r);
    let mut ips: Vec<String> = ranked.into_iter().map(|(_, ip)| ip).collect();
    ips.dedup();
    if ips.is_empty() {
        ips.push("127.0.0.1".into());
    }
    ips
}

/// Start or stop the server to match the setting.
fn manage(mut console: ResMut<Console>, inilla: Res<Inilla>) {
    let d = &inilla.settings.debug;
    let want = d.remote_console && !d.console_token.is_empty();
    let matches = console
        .running
        .as_ref()
        .is_some_and(|r| r.port == d.console_port && r.token == d.console_token);
    if want && matches || !want && console.running.is_none() {
        return;
    }
    if let Some(r) = console.running.take() {
        r.stop.store(true, Ordering::SeqCst);
        info!("inilla: remote console stopped");
    }
    if !want {
        return;
    }
    let listener = match TcpListener::bind(("0.0.0.0", d.console_port)) {
        Ok(l) => l,
        Err(e) => {
            warn!(
                "inilla: remote console could not listen on {}: {e}",
                d.console_port
            );
            return;
        }
    };
    let _ = listener.set_nonblocking(true);
    let stop = Arc::new(AtomicBool::new(false));
    let ctx = Arc::new(ServerCtx {
        token: d.console_token.clone(),
        tx: console.tx.clone(),
        state: console.state.clone(),
        root: crate::local_state::home(),
    });
    let stop2 = stop.clone();
    let _ = std::thread::Builder::new()
        .name("inilla-console".into())
        .spawn(move || accept_loop(listener, ctx, stop2));
    for ip in local_addresses() {
        info!("inilla: remote console at http://{ip}:{}/", d.console_port);
    }
    console.running = Some(Running {
        port: d.console_port,
        token: d.console_token.clone(),
        stop,
    });
}

struct ServerCtx {
    token: String,
    tx: Sender<ConsoleCmd>,
    state: Arc<Mutex<String>>,
    root: Option<PathBuf>,
}

fn accept_loop(listener: TcpListener, ctx: Arc<ServerCtx>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, peer)) => {
                let _ = stream.set_nonblocking(false);
                let ctx = ctx.clone();
                let _ = std::thread::Builder::new()
                    .name("inilla-console-conn".into())
                    .spawn(move || {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                        let Ok(read) = stream.try_clone() else { return };
                        let mut reader = std::io::BufReader::new(read);
                        if let Ok(mut req) = http::read_request(&mut reader) {
                            req.peer = peer.to_string();
                            handle(&ctx, req, &mut stream);
                        }
                    });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}

/// `path` under `root`, refusing anything that would climb out of it.
fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel.trim_start_matches('/'));
    let mut out = root.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

fn to_json(v: &PlainValue) -> serde_json::Value {
    match v {
        PlainValue::Null => serde_json::Value::Null,
        PlainValue::Bool(b) => (*b).into(),
        PlainValue::Num(n) => {
            serde_json::Number::from_f64(*n).map_or(serde_json::Value::Null, Into::into)
        }
        PlainValue::Str(s) => s.clone().into(),
        PlainValue::List(l) => l.iter().map(to_json).collect::<Vec<_>>().into(),
        PlainValue::Map(m) => {
            serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), to_json(v))).collect())
        }
    }
}

fn handle(ctx: &ServerCtx, req: Request, w: &mut std::net::TcpStream) {
    if req.method == "GET" && (req.path == "/" || req.path == "/index.html") {
        let _ = http::respond(w, 200, "text/html; charset=utf-8", BRANDED.as_bytes());
        return;
    }
    if !req.token().is_some_and(|t| http::token_eq(t, &ctx.token)) {
        let _ = http::respond_text(w, 401, "token required");
        return;
    }
    let root = ctx.root.clone();
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/state") => {
            let s = ctx.state.lock().map(|s| s.clone()).unwrap_or_default();
            let _ = http::respond_json(w, 200, &s);
        }
        ("GET", "/api/log") => {
            let since: u64 = req.query("since").and_then(|s| s.parse().ok()).unwrap_or(0);
            let ring = inilla_core::logring::global();
            let lines = if since == 0 {
                ring.tail(3000)
            } else if req.query("wait").is_some() {
                ring.wait_since(since, Duration::from_secs(20))
            } else {
                ring.since(since)
            };
            let next = lines
                .last()
                .map_or(since.max(ring.next_seq()), |(s, _)| s + 1);
            let body = serde_json::json!({ "next": next, "lines": lines.iter().map(|(_, l)| l).collect::<Vec<_>>() });
            let _ = http::respond_json(w, 200, &body.to_string());
        }
        ("POST", "/api/lua") => {
            let chunk = String::from_utf8_lossy(&req.body).into_owned();
            let (tx, rx) = crossbeam_channel::bounded(1);
            let _ = ctx.tx.send(ConsoleCmd::Lua { chunk, reply: tx });
            let body = match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Ok(values)) => {
                    serde_json::json!({ "ok": true, "values": values.iter().map(to_json).collect::<Vec<_>>() })
                }
                Ok(Err(e)) => serde_json::json!({ "ok": false, "error": e }),
                Err(_) => {
                    serde_json::json!({ "ok": false, "error": "the game did not answer in 10 s" })
                }
            };
            let _ = http::respond_json(w, 200, &body.to_string());
        }
        ("POST", "/api/cmd") => {
            let v: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            let op = v["op"].as_str().unwrap_or("");
            let cmd = match op {
                "press" => v["cmd"].as_str().map(|c| ConsoleCmd::Press(c.to_string())),
                "chat" => v["text"].as_str().map(|t| ConsoleCmd::Chat(t.to_string())),
                "screenshot" => Some(ConsoleCmd::Press("SCREENSHOT".into())),
                _ => None,
            };
            match cmd {
                Some(c) => {
                    let _ = ctx.tx.send(c);
                    let _ = http::respond_json(w, 200, "{\"ok\":true}");
                }
                None => {
                    let _ = http::respond_json(w, 400, "{\"ok\":false,\"error\":\"unknown op\"}");
                }
            }
        }
        ("GET", "/api/files") => {
            let Some(dir) = root.and_then(|r| safe_join(&r, req.query("path").unwrap_or("")))
            else {
                let _ = http::respond_text(w, 404, "no such folder");
                return;
            };
            let mut entries = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let meta = e.metadata().ok();
                    entries.push(serde_json::json!({
                        "name": e.file_name().to_string_lossy(),
                        "dir": meta.as_ref().is_some_and(|m| m.is_dir()),
                        "size": meta.as_ref().map_or(0, |m| m.len()),
                        "modified": meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs()),
                    }));
                }
            }
            let _ = http::respond_json(w, 200, &serde_json::Value::from(entries).to_string());
        }
        ("GET", "/api/file") => {
            let Some(path) = root.and_then(|r| safe_join(&r, req.query("path").unwrap_or("")))
            else {
                let _ = http::respond_text(w, 404, "no such file");
                return;
            };
            match std::fs::File::open(&path) {
                Ok(mut f) => {
                    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let disposition = format!("attachment; filename=\"{name}\"");
                    let _ = http::write_head(
                        w,
                        &http::Head {
                            status: 200,
                            content_type: "application/octet-stream",
                            length: Some(len),
                            extra: &[("Content-Disposition", disposition)],
                        },
                    );
                    let _ = http::copy_n(&mut f, w, len);
                }
                Err(_) => {
                    let _ = http::respond_text(w, 404, "no such file");
                }
            }
        }
        ("PUT", "/api/file") => {
            let Some(path) = root.and_then(|r| safe_join(&r, req.query("path").unwrap_or("")))
            else {
                let _ = http::respond_text(w, 400, "bad path");
                return;
            };
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(&path, &req.body) {
                Ok(()) => {
                    info!(
                        "inilla: console wrote {} ({} bytes)",
                        path.display(),
                        req.body.len()
                    );
                    let _ = http::respond_json(w, 200, "{\"ok\":true}");
                }
                Err(e) => {
                    let _ = http::respond_text(w, 500, &e.to_string());
                }
            }
        }
        ("DELETE", "/api/file") => {
            let Some(path) = root.and_then(|r| safe_join(&r, req.query("path").unwrap_or("")))
            else {
                let _ = http::respond_text(w, 400, "bad path");
                return;
            };
            let ok = std::fs::remove_file(&path).is_ok();
            let _ = http::respond_json(w, if ok { 200 } else { 404 }, &format!("{{\"ok\":{ok}}}"));
        }
        _ => {
            let _ = http::respond_text(w, 404, "no such endpoint");
        }
    }
}

/// Hand the console's commands to the game, on the main thread.
fn service(console: Res<Console>, mut synth: ResMut<Synth>) {
    for cmd in console.rx.try_iter() {
        match cmd {
            ConsoleCmd::Lua { chunk, reply } => synth.lua(LuaJob {
                chunk,
                reply: Some(reply),
            }),
            ConsoleCmd::Press(name) => {
                synth.press(PadBinding::cmd(&name));
                synth.release(PadBinding::cmd(&name));
            }
            ConsoleCmd::Chat(line) => synth.chat(line),
        }
    }
}

/// Once a second: what the console's status panel shows.
fn publish_state(
    console: Res<Console>,
    inilla: Res<Inilla>,
    client: Res<State<ClientState>>,
    build: Res<benilla_world::build_id::BuildId>,
    data: Res<super::data::DataState>,
    pad: Res<super::pad::PadHud>,
    time: Res<Time<Real>>,
    mut next: Local<f64>,
    mut fps: Local<f32>,
) {
    let dt = time.delta_secs().max(1e-4);
    *fps = if *fps == 0.0 {
        1.0 / dt
    } else {
        *fps * 0.95 + (1.0 / dt) * 0.05
    };
    let now = time.elapsed_secs_f64();
    if console.running.is_none() || now < *next {
        return;
    }
    *next = now + 1.0;
    let state = serde_json::json!({
        "build": format!("{} ({}, {})", build.short, build.date, build.profile),
        "device": inilla_platform::device::describe(),
        "state": format!("{:?}", client.get()),
        "fps": (*fps * 10.0).round() / 10.0,
        "memory_free_mb": inilla_platform::device::available_memory().map(|b| b / 1_048_576),
        "thermal": format!("{:?}", inilla_platform::device::thermal_state()),
        "uptime_s": now.round(),
        "server": inilla.launcher.current_server().map(|s| s.realmlist()),
        "data": { "archives": data.status.present.len(), "complete": data.status.complete(), "gb": data.status.total_bytes() as f64 / 1e9 },
        "controller": pad.name,
        "touch_view": inilla.layout.active_view,
    });
    if let Ok(mut s) = console.state.lock() {
        *s = state.to_string();
    }
}

/// The console page: one file, no build step.
const PAGE: &str = include_str!("console.html");

/// Vanillish: the page under the app's own name — console.html stays iNilla's file, and on iOS
/// this is that page unchanged.
static BRANDED: LazyLock<String> = LazyLock::new(|| branded(PAGE, super::ui::words::APP));

/// `page` with iNilla's name (`iNilla`, and `INILLA` in the header) replaced by `app`. Only the
/// name: the page's own keys (`inilla-token`) and the log's `inilla:` prefix stay.
fn branded(page: &str, app: &str) -> String {
    page.replace("iNilla", app)
        .replace("INILLA", &app.to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_comes_first_and_cellular_never() {
        assert_eq!(iface_rank("en0"), Some(0));
        assert_eq!(iface_rank("wlan0"), Some(0));
        assert_eq!(iface_rank("wlan1"), Some(0));
        assert_eq!(iface_rank("swlan0"), Some(1));
        assert_eq!(iface_rank("ap0"), Some(1));
        assert_eq!(iface_rank("rndis0"), Some(1));
        assert_eq!(iface_rank("ncm0"), Some(1));
        assert_eq!(iface_rank("tun0"), Some(3));
        assert_eq!(iface_rank("wg0"), Some(3));
        assert_eq!(iface_rank("eth0"), Some(2));
        assert_eq!(iface_rank("pdp_ip0"), Some(2));
        // A desktop's wired LAN before its VPN; iOS's VPN (`utun`) is no `tun`.
        assert!(iface_rank("enp3s0") < iface_rank("wg0"));
        assert_eq!(iface_rank("utun3"), Some(2));
        assert_eq!(iface_rank("rmnet_data0"), None);
        assert_eq!(iface_rank("v4-rmnet_data0"), None);
        assert_eq!(iface_rank("rmnet0"), None);
        assert_eq!(iface_rank("ccmni1"), None);
        assert_eq!(iface_rank("dummy0"), None);
    }

    /// Vanillish: Windows' adapter names, Wi-Fi first and the virtual ones last.
    #[test]
    fn windows_adapters_rank_by_name() {
        assert_eq!(adapter_rank("Wi-Fi"), 0);
        assert_eq!(adapter_rank("Wi-Fi 2"), 0);
        assert_eq!(adapter_rank("WLAN"), 0);
        assert_eq!(adapter_rank("Wireless Network Connection"), 0);
        assert_eq!(adapter_rank("Ethernet"), 1);
        assert_eq!(adapter_rank("Ethernet 3"), 1);
        assert_eq!(adapter_rank("Bluetooth Network Connection"), 2);
        assert_eq!(adapter_rank("Local Area Connection* 10"), 2);
        for virtual_one in [
            "vEthernet (WSL)",
            "vEthernet (Default Switch)",
            "vEthernet (WSL (Hyper-V firewall))",
            "Hyper-V Virtual Ethernet Adapter",
            "DockerNAT",
            "VirtualBox Host-Only Network",
            "VMware Network Adapter VMnet8",
            "Tailscale",
            "ZeroTier One [8056c2e21c000001]",
            "WireGuard Tunnel",
            "OpenVPN Data Channel Offload",
            "TAP-Windows Adapter V9",
            "NordVPN",
            "Loopback Pseudo-Interface 1",
        ] {
            assert_eq!(adapter_rank(virtual_one), 3, "{virtual_one}");
        }
        // A word that only holds "tap" is no TAP adapter.
        assert_eq!(adapter_rank("Startap Ethernet"), 1);
    }

    #[test]
    fn the_page_carries_the_app_name() {
        let page = branded(PAGE, "Vanillish");
        assert!(page.contains("<title>Vanillish console</title>"));
        assert!(page.contains("<b>VANILLISH</b>"));
        assert!(!page.contains("iNilla") && !page.contains("INILLA"));
        // The storage key and the log colouring are not the brand.
        assert!(page.contains("inilla-token"));
        assert_eq!(branded(PAGE, "iNilla"), PAGE);
    }
}
