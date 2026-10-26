//! Stamps the commit this exe was built from, exactly as the `benilla` launcher shim does
//! (`benilla-buildstamp` owns the rule). For a Windows target it also gives the main thread an
//! 8 MB stack, and for the shipping MSVC build it embeds the exe's icon, manifest and version
//! (`windows/vanillish.rc`); the Linux gnu check stays free of a resource compiler.

use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    benilla_buildstamp::emit();

    let cfg = |key: &str| env::var(key).unwrap_or_default();
    if cfg("CARGO_CFG_TARGET_OS") != "windows" {
        return;
    }
    // A Windows exe's main thread gets the stack its header asks for, 1 MB by default, against
    // 8 MB on macOS and Linux; Bevy runs Update, PostUpdate and Extract on that thread.
    let msvc = cfg("CARGO_CFG_TARGET_ENV") == "msvc";
    if msvc {
        println!("cargo::rustc-link-arg-bins=/STACK:8388608");
    } else {
        println!("cargo::rustc-link-arg-bins=-Wl,--stack,8388608");
    }
    if msvc {
        resources();
    }
}

/// `windows/vanillish.rc` compiled (rc.exe) and linked into the exe, with the version as
/// preprocessor defines: `$VANILLISH_VERSION_NAME` for a release, else the workspace version and
/// the commit, as the APK's versionName; the fourth number is `$VANILLISH_CI_BUILD`, else 0.
fn resources() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let windows = Path::new(&manifest_dir)
        .ancestors()
        .nth(2)
        .expect("the crate sits at crates/vanillish-windows")
        .join("windows");
    for file in ["vanillish.rc", "vanillish.manifest", "Vanillish.ico"] {
        println!("cargo::rerun-if-changed={}", windows.join(file).display());
    }
    println!("cargo::rerun-if-env-changed=VANILLISH_VERSION_NAME");
    println!("cargo::rerun-if-env-changed=VANILLISH_CI_BUILD");

    let commit = Command::new("git")
        .arg("-C")
        .arg(&manifest_dir)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    let version = env::var("VANILLISH_VERSION_NAME")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}+{commit}", env!("CARGO_PKG_VERSION")));
    let [major, minor, patch] = version_numbers(&version);
    let build = env::var("VANILLISH_CI_BUILD")
        .ok()
        .and_then(|b| b.parse::<u16>().ok())
        .unwrap_or(0);

    // The strings go into the .rc as C string literals, quotes included.
    let macros = [
        format!("VANILLISH_VERSION_MAJOR={major}"),
        format!("VANILLISH_VERSION_MINOR={minor}"),
        format!("VANILLISH_VERSION_PATCH={patch}"),
        format!("VANILLISH_VERSION_BUILD={build}"),
        format!("VANILLISH_VERSION_STR=\"{version}\""),
        format!("VANILLISH_COMMIT=\"{commit}\""),
    ];
    // The icon and manifest are found beside the .rc (the include path), wherever rc.exe runs.
    embed_resource::compile(
        windows.join("vanillish.rc"),
        embed_resource::ParamsMacrosAndIncludeDirs(&macros, [&windows]),
    )
    .manifest_required()
    .unwrap();
}

/// `1.2.3`, `1.2.3-rc1`, `0.2.0+abc1234` → `[1, 2, 3]`: the three leading numbers VERSIONINFO
/// carries. Any other shape, or a character that would end the .rc's string literal, fails the
/// build rather than stamp a wrong version.
fn version_numbers(version: &str) -> [u16; 3] {
    let plain = version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let mut numbers = core.split('.').map(|n| n.parse::<u16>().ok());
    match (numbers.next(), numbers.next(), numbers.next()) {
        (Some(Some(major)), Some(Some(minor)), Some(Some(patch))) if plain => [major, minor, patch],
        _ => panic!(
            "vanillish-windows: version {version:?} is not MAJOR.MINOR.PATCH[-suffix] \
             (VANILLISH_VERSION_NAME, or the workspace version)"
        ),
    }
}
