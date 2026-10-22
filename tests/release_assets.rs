//! install.sh derives an asset name from `uname` and release.yml publishes a fixed list.
//! If those drift, every install 404s while the rest of the suite stays green, so the
//! asset name is read out of the real script, per platform, by stubbing `uname`.
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::Env;

const PLATFORMS: [(&str, &str, &str); 4] = [
    ("Linux", "x86_64", "apm-linux-x64"),
    ("Linux", "aarch64", "apm-linux-arm64"),
    ("Darwin", "x86_64", "apm-darwin-x64"),
    ("Darwin", "arm64", "apm-darwin-arm64"),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap()
}

fn have_sh() -> bool {
    Command::new("sh").args(["-c", "true"]).status().is_ok()
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
    let mut expected: Vec<String> = PLATFORMS.iter().map(|p| p.2.to_string()).collect();
    expected.push("apm-windows-x64.exe".into());
    expected.sort();
    assert_eq!(published_assets(), expected);
}

#[test]
fn install_sh_requests_a_published_asset_on_every_platform() {
    if !have_sh() {
        eprintln!("skipped: no sh on PATH");
        return;
    }
    let published = published_assets();
    for (s, m, expected) in PLATFORMS {
        let asset = asset_requested_on(s, m);
        assert_eq!(asset, expected, "{s}/{m}");
        assert!(published.contains(&asset), "{asset} not in release.yml");
    }
}

#[test]
fn each_asset_ships_a_checksum_install_sh_verifies() {
    // install.sh fetches "<asset>.sha256" and refuses to install on a mismatch.
    assert!(read(".github/workflows/release.yml").contains("> ${{ matrix.asset }}.sha256"));
    assert!(read("install.sh").contains("$ASSET.sha256"));
}

#[test]
fn uninstall_keeps_the_database() {
    if !have_sh() {
        eprintln!("skipped: no sh on PATH");
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
