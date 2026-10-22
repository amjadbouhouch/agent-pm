//! install.ps1 run for real by Windows PowerShell against a local mirror holding a copy of
//! the test binary. Each test gets its own home and its own registry key standing in for the
//! user PATH (`APM_PATH_REG_KEY`), so the machine's PATH and ~\.agent-pm are never touched.
#![cfg(windows)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

const ASSET: &str = "apm-windows-x64.exe";

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("install.ps1")
}

fn apm_version() -> String {
    format!("apm {}", env!("CARGO_PKG_VERSION"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A mirror (`srv`), a home (`USERPROFILE`) and a registry key for the user PATH.
struct Setup {
    tmp: TempDir,
    key: String,
}

impl Setup {
    fn new(name: &str) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("srv")).unwrap();
        std::fs::create_dir_all(tmp.path().join("home")).unwrap();
        let key = format!(r"Software\agent-pm-test-{}-{name}", std::process::id());
        let s = Setup { tmp, key };
        s.serve(&std::fs::read(env!("CARGO_BIN_EXE_apm")).unwrap());
        s
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
    }

    fn srv(&self) -> PathBuf {
        self.path("srv")
    }

    fn bin(&self) -> PathBuf {
        self.path("bin")
    }

    /// The default install dir under this setup's home.
    fn default_bin(&self) -> PathBuf {
        self.path(r"home\.agent-pm\bin")
    }

    /// Publishes `bytes` as the asset, with a matching checksum file.
    fn serve(&self, bytes: &[u8]) {
        let asset = self.srv().join(ASSET);
        std::fs::write(&asset, bytes).unwrap();
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "(Get-FileHash -LiteralPath '{}' -Algorithm SHA256).Hash.ToLower()",
                asset.display()
            ))
            .output()
            .unwrap();
        let hash = text(&out.stdout).trim().to_string();
        assert_eq!(hash.len(), 64, "{}", text(&out.stderr));
        self.write_sum(&format!("{hash}  {ASSET}\n"));
    }

    fn write_sum(&self, content: &str) {
        std::fs::write(self.srv().join(format!("{ASSET}.sha256")), content).unwrap();
    }

    /// Environment every run gets: this setup's home and PATH key, no APM_* from the caller.
    fn env(&self, cmd: &mut Command) {
        for var in [
            "APM_VERSION",
            "APM_INSTALL_DIR",
            "APM_REPO",
            "APM_BASE_URL",
            "APM_MODIFY_PATH",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("USERPROFILE", self.path("home"))
            .env("APM_PATH_REG_KEY", &self.key)
            .env("NO_COLOR", "1");
    }

    /// `install.ps1 <args>` run as a file, non-interactively.
    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script())
        .args(args)
        .stdin(Stdio::null());
        self.env(&mut cmd);
        cmd.output().unwrap()
    }

    /// Installs from the mirror into `bin` without touching the PATH, and asserts success.
    fn install(&self) -> Output {
        let out = self.run(&[
            "-BaseUrl",
            self.srv().to_str().unwrap(),
            "-Dir",
            self.bin().to_str().unwrap(),
            "-NoModifyPath",
        ]);
        assert_success(&out);
        out
    }

    /// The user PATH value in this setup's key, as stored (unexpanded); None when unset.
    fn user_path(&self) -> Option<String> {
        let out = Command::new("reg")
            .args(["query", &format!(r"HKCU\{}", self.key), "/v", "Path"])
            .output()
            .unwrap();
        let stdout = text(&out.stdout);
        let line = stdout.lines().find(|l| l.contains("REG_"))?;
        let value = line.split("REG_").nth(1).unwrap();
        Some(
            value
                .split_once(char::is_whitespace)
                .map_or("", |v| v.1)
                .trim()
                .to_string(),
        )
    }

    fn set_user_path(&self, value: &str) {
        let key = format!(r"HKCU\{}", self.key);
        let args = [
            "add",
            &key,
            "/v",
            "Path",
            "/t",
            "REG_EXPAND_SZ",
            "/d",
            value,
            "/f",
        ];
        assert!(
            Command::new("reg")
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = Command::new("reg")
            .args(["delete", &format!(r"HKCU\{}", self.key), "/f"])
            .output();
    }
}

fn assert_success(out: &Output) {
    assert!(
        out.status.success(),
        "install.ps1 failed\nstdout:\n{}\nstderr:\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
}

fn assert_failure(out: &Output, message: &str) {
    assert_eq!(out.status.code(), Some(1), "stdout:\n{}", text(&out.stdout));
    assert!(
        text(&out.stderr).contains(message),
        "stderr:\n{}",
        text(&out.stderr)
    );
}

fn version_of(exe: &Path) -> String {
    let out = Command::new(exe).arg("--version").output().unwrap();
    text(&out.stdout).trim().to_string()
}

/// File names in `dir`, sorted.
fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn installs_a_binary_that_runs() {
    let s = Setup::new("install");
    let out = s.install();
    let stdout = text(&out.stdout);
    assert!(stdout.contains("checksum  verified"), "{stdout}");
    assert!(
        stdout.contains(&format!("installed {}", apm_version())),
        "{stdout}"
    );
    assert_eq!(version_of(&s.bin().join("apm.exe")), apm_version());
    assert_eq!(files(&s.bin()), ["apm.exe"], "staging files left behind");
    assert_eq!(s.user_path(), None, "-NoModifyPath must not touch the PATH");

    let again = s.install();
    assert!(text(&again.stdout).contains(&format!("reinstalled {}", apm_version())));
}

#[test]
fn a_bad_checksum_is_refused_and_keeps_the_installed_binary() {
    let s = Setup::new("badsum");
    s.install();
    s.serve(b"not apm");
    s.write_sum(&format!("{}  {ASSET}\n", "0".repeat(64)));

    let out = s.run(&[
        "-BaseUrl",
        s.srv().to_str().unwrap(),
        "-Dir",
        s.bin().to_str().unwrap(),
        "-NoModifyPath",
    ]);
    assert_failure(&out, "checksum mismatch");
    assert_eq!(files(&s.bin()), ["apm.exe"]);
    assert_eq!(version_of(&s.bin().join("apm.exe")), apm_version());
}

#[test]
fn a_binary_that_does_not_run_is_refused() {
    let s = Setup::new("norun");
    s.serve(b"MZ but not a program");
    let out = s.run(&[
        "-BaseUrl",
        s.srv().to_str().unwrap(),
        "-Dir",
        s.bin().to_str().unwrap(),
        "-NoModifyPath",
    ]);
    assert_failure(&out, "failed to run");
    assert_eq!(files(&s.bin()), Vec::<String>::new());
}

#[test]
fn modify_path_adds_once_and_uninstall_removes_it_and_keeps_the_database() {
    let s = Setup::new("path");
    s.set_user_path(r"C:\keep;%SystemRoot%\keep");
    let data = s.path(r"home\.agent-pm");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("apm.db"), "keep me").unwrap();

    // Default dir, so the PATH entry is written relative to %USERPROFILE%.
    for _ in 0..2 {
        assert_success(&s.run(&["-BaseUrl", s.srv().to_str().unwrap(), "-ModifyPath"]));
    }
    assert_eq!(
        s.user_path().as_deref(),
        Some(r"C:\keep;%SystemRoot%\keep;%USERPROFILE%\.agent-pm\bin")
    );
    assert_eq!(version_of(&s.default_bin().join("apm.exe")), apm_version());

    let out = s.run(&["-Uninstall"]);
    assert_success(&out);
    assert!(!s.default_bin().exists(), "empty bin dir should be removed");
    assert_eq!(
        std::fs::read_to_string(data.join("apm.db")).unwrap(),
        "keep me"
    );
    assert_eq!(s.user_path().as_deref(), Some(r"C:\keep;%SystemRoot%\keep"));
}

#[test]
fn without_a_terminal_the_path_is_left_alone() {
    let s = Setup::new("noask");
    let out = s.run(&[
        "-BaseUrl",
        s.srv().to_str().unwrap(),
        "-Dir",
        s.bin().to_str().unwrap(),
    ]);
    assert_success(&out);
    assert!(text(&out.stdout).contains("-ModifyPath"));
    assert_eq!(s.user_path(), None);
}

#[test]
fn uninstall_without_modify_path_keeps_the_path_entry() {
    let s = Setup::new("unkeep");
    s.install();
    s.set_user_path(&s.bin().display().to_string());
    assert_success(&s.run(&[
        "-Uninstall",
        "-Dir",
        s.bin().to_str().unwrap(),
        "-NoModifyPath",
    ]));
    assert!(!s.bin().join("apm.exe").exists());
    assert_eq!(s.user_path(), Some(s.bin().display().to_string()));
}

#[test]
fn upgrading_while_apm_runs_moves_the_old_binary_aside() {
    let s = Setup::new("running");
    s.install();
    // `-b -` waits for stdin to close, so this apm.exe keeps running.
    let mut running = Command::new(s.bin().join("apm.exe"))
        .args([
            "--db",
            s.path("x.db").to_str().unwrap(),
            "add",
            "x",
            "-b",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let out = s.install();
    assert!(
        text(&out.stdout).contains("still running"),
        "{}",
        text(&out.stdout)
    );
    assert_eq!(version_of(&s.bin().join("apm.exe")), apm_version());

    drop(running.stdin.take());
    running.wait().unwrap();
    s.install();
    assert_eq!(files(&s.bin()), ["apm.exe"], "old copy not cleaned up");
}

#[test]
fn a_binary_held_open_is_reported() {
    let s = Setup::new("locked");
    s.install();
    let exe = s.bin().join("apm.exe");
    // Opened without sharing: it cannot be renamed, deleted or replaced.
    let mut holder = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "$f = [IO.File]::Open('{}', 'Open', 'Read', 'None'); 'locked'; [Console]::In.ReadLine()",
            exe.display()
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(holder.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "locked");

    let out = s.run(&[
        "-BaseUrl",
        s.srv().to_str().unwrap(),
        "-Dir",
        s.bin().to_str().unwrap(),
        "-NoModifyPath",
    ]);
    drop(holder.stdin.take());
    holder.wait().unwrap();
    assert_failure(&out, "holds it open");
    assert_eq!(files(&s.bin()), ["apm.exe"]);
    assert_eq!(version_of(&exe), apm_version());
}

/// `irm ... | iex`: no file, no parameters, settings from the environment, and a failure
/// must not end the caller's session.
#[test]
fn runs_under_iex_from_environment_variables() {
    let s = Setup::new("iex");
    let iex = |base: &Path| {
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "Get-Content -Raw -LiteralPath '{}' | Invoke-Expression; 'still here'",
                script().display()
            ))
            .stdin(Stdio::null());
        s.env(&mut cmd);
        cmd.env("APM_BASE_URL", base)
            .env("APM_INSTALL_DIR", s.bin())
            .env("APM_MODIFY_PATH", "yes")
            .output()
            .unwrap()
    };

    let out = iex(&s.srv());
    assert_success(&out);
    assert!(text(&out.stdout).contains("still here"));
    assert_eq!(version_of(&s.bin().join("apm.exe")), apm_version());
    assert_eq!(s.user_path(), Some(s.bin().display().to_string()));

    let out = iex(&s.path("missing"));
    assert!(
        text(&out.stderr).contains("download failed"),
        "{}",
        text(&out.stderr)
    );
    assert!(
        text(&out.stdout).contains("still here"),
        "a failure closed the session"
    );
}
