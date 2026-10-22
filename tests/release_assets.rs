//! install.sh derives an asset name from `uname` and release.yml publishes a fixed list.
//! If those drift, every install 404s while the rest of the suite stays green, so the
//! asset name is read out of the real script, per platform, by stubbing `uname`.
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::Env;

/// A platform as `uname -s` / `uname -m` report it, and the asset install.sh must ask for.
struct Platform {
    os: &'static str,
    arch: &'static str,
    asset: &'static str,
}

const PLATFORMS: [Platform; 4] = [
    Platform {
        os: "Linux",
        arch: "x86_64",
        asset: "apm-linux-x64",
    },
    Platform {
        os: "Linux",
        arch: "aarch64",
        asset: "apm-linux-arm64",
    },
    Platform {
        os: "Darwin",
        arch: "x86_64",
        asset: "apm-darwin-x64",
    },
    Platform {
        os: "Darwin",
        arch: "arm64",
        asset: "apm-darwin-arm64",
    },
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap()
}

/// Whether the install.sh tests can run. `sh` is always there on Linux and macOS, so a
/// missing one fails the test there; only Windows without Git Bash is skipped.
fn have_sh() -> bool {
    let found = Command::new("sh").args(["-c", "true"]).status().is_ok();
    assert!(found || cfg!(windows), "sh not found on PATH");
    if !found {
        eprintln!("skipped: no sh on PATH");
    }
    found
}

/// `PATH` with `dir` first.
fn path_with(dir: &Path) -> std::ffi::OsString {
    let old = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![dir.to_path_buf()];
    dirs.extend(std::env::split_paths(&old));
    std::env::join_paths(dirs).unwrap()
}

/// Runs install.sh with a stubbed `uname` and returns the asset it requests.
fn asset_requested_on(uname_s: &str, uname_m: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let stub = tmp.path().join("uname");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  -s) echo {uname_s} ;;\n  -m) echo {uname_m} ;;\nesac\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let out = Command::new("sh")
        .arg(root().join("install.sh"))
        .current_dir(tmp.path())
        .env("PATH", path_with(tmp.path()))
        .env("HOME", tmp.path())
        .env("NO_COLOR", "1")
        // Closed port: the script prints the asset it wants, then fails fast.
        .env("APM_BASE_URL", "http://127.0.0.1:1")
        .env("APM_INSTALL_DIR", tmp.path().join("bin"))
        .output()
        .unwrap();
    assert!(!out.status.success(), "install.sh should fail to download");
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("source"))
        .and_then(|url| url.trim().rsplit('/').next())
        .unwrap_or_else(|| panic!("install.sh printed no asset name:\n{stdout}"))
        .to_string()
}

/// `asset: <name>` entries of release.yml's build matrix.
fn published_assets() -> Vec<String> {
    let mut assets: Vec<String> = read(".github/workflows/release.yml")
        .lines()
        .filter_map(|l| l.trim().strip_prefix("asset:"))
        .map(|a| a.trim().to_string())
        .collect();
    assets.sort();
    assets
}

#[test]
fn release_publishes_every_unix_asset_and_the_windows_exe() {
    let mut expected: Vec<String> = PLATFORMS.iter().map(|p| p.asset.to_string()).collect();
    expected.push("apm-windows-x64.exe".into());
    expected.sort();
    assert_eq!(published_assets(), expected);
}

#[test]
fn install_sh_requests_a_published_asset_on_every_platform() {
    if !have_sh() {
        return;
    }
    let published = published_assets();
    for p in PLATFORMS {
        let asset = asset_requested_on(p.os, p.arch);
        assert_eq!(asset, p.asset, "{}/{}", p.os, p.arch);
        assert!(published.contains(&asset), "{asset} not in release.yml");
    }
}

#[test]
fn each_asset_ships_a_checksum_the_installers_verify() {
    // The installers fetch "<asset>.sha256" and refuse to install on a mismatch.
    assert!(read(".github/workflows/release.yml").contains("> ${{ matrix.asset }}.sha256"));
    assert!(read("install.sh").contains("$ASSET.sha256"));
    assert!(read("install.ps1").contains("\"$asset.sha256\""));
}

/// Runs install.ps1 against an empty mirror and returns the asset it requests.
#[cfg(windows)]
fn asset_requested_by_install_ps1() -> String {
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(root().join("install.ps1"))
        .arg("-BaseUrl")
        .arg(tmp.path())
        .arg("-Dir")
        .arg(tmp.path().join("bin"))
        .arg("-NoModifyPath")
        .env("USERPROFILE", tmp.path())
        .env("APM_PATH_REG_KEY", r"Software\agent-pm-test-unused")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(!out.status.success(), "install.ps1 should find no asset");
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("source"))
        .and_then(|src| src.trim().rsplit(['/', '\\']).next())
        .unwrap_or_else(|| panic!("install.ps1 printed no asset name:\n{stdout}"))
        .to_string()
}

#[test]
fn install_ps1_requests_the_published_windows_asset() {
    #[cfg(windows)]
    let asset = asset_requested_by_install_ps1();
    // No Windows PowerShell elsewhere: the name the script spells out.
    #[cfg(not(windows))]
    let asset = String::from("apm-windows-x64.exe");
    assert!(read("install.ps1").contains(&format!("'{asset}'")));
    assert!(
        published_assets().contains(&asset),
        "{asset} not in release.yml"
    );
}

#[test]
fn install_ps1_is_ascii() {
    // Windows PowerShell 5.1 reads a script without a BOM in the ANSI code page.
    let script = read("install.ps1");
    let bad = script.lines().position(|l| !l.is_ascii());
    assert_eq!(
        bad,
        None,
        "non-ASCII on line {:?} of install.ps1",
        bad.map(|n| n + 1)
    );
}

#[test]
fn uninstall_keeps_the_database() {
    if !have_sh() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".agent-pm");
    std::fs::create_dir_all(data.join("bin")).unwrap();
    std::fs::write(data.join("bin/apm"), "").unwrap();
    std::fs::write(data.join("apm.db"), "keep me").unwrap();

    let out = Command::new("sh")
        .arg(root().join("install.sh"))
        .arg("--uninstall")
        .env("HOME", home.path())
        .env_remove("APM_INSTALL_DIR")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !data.join("bin").exists(),
        "empty bin dir should be removed"
    );
    assert_eq!(
        std::fs::read_to_string(data.join("apm.db")).unwrap(),
        "keep me"
    );
}

/// The `version = "..."` line right after `anchor` (Cargo.toml's `[package]`, or the
/// apm entry of Cargo.lock).
fn version_after(text: &str, anchor: &str) -> String {
    let rest = &text[text.find(anchor).unwrap_or_else(|| panic!("no {anchor:?}"))..];
    rest.lines()
        .find_map(|l| l.strip_prefix("version = "))
        .unwrap()
        .trim_matches('"')
        .to_string()
}

#[test]
fn cargo_toml_lock_and_binary_agree_on_the_version() {
    let toml = version_after(&read("Cargo.toml"), "[package]");
    let lock = version_after(&read("Cargo.lock"), "name = \"apm\"");
    assert_eq!(
        lock, toml,
        "Cargo.lock's apm entry is out of step with Cargo.toml"
    );

    let out = Env::new().apm().arg("--version").assert().success();
    let printed = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(printed.trim(), format!("apm {toml}"));
}
