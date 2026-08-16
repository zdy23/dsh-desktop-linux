//! DeepSeek Harness desktop shell.
//!
//! A thin Tauri wrapper that:
//!   1. spawns `dsh web` on 127.0.0.1 (or connects to one already running),
//!   2. waits for the HTTP server to accept connections,
//!   3. navigates the app window to the GUI URL,
//!   4. kills the spawned child when the window closes.
//!
//! Configuration via environment variables:
//!   DSH_BIN  – path to the `dsh` executable (default: `dsh` on PATH)
//!   DSH_HOST – bind host (default: 127.0.0.1)
//!   DSH_PORT – bind port (default: 3080)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use tauri::{AppHandle, Manager, RunEvent};

/// Holds the `dsh web` child process we spawned, if any.
struct DshChild(Mutex<Option<Child>>);

/// Kill and reap the spawned `dsh` child, if any.
fn kill_child(app: &AppHandle) {
    if let Some(state) = app.try_state::<DshChild>() {
        let child = state.0.lock().unwrap().take();
        if let Some(mut child) = child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn main() {
    let app = tauri::Builder::default()
        .manage(DshChild(Mutex::new(None)))
        .setup(|app| {
            let handle = app.handle().clone();
            // Boot the server off the UI thread so the window can appear
            // immediately with the splash page.
            thread::spawn(move || {
                if let Err(err) = boot_and_connect(&handle) {
                    eprintln!("[dsh-desktop] {err}");
                    show_error(&handle, &err);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            kill_child(app_handle);
        }
    });
}

/// Spawn/connect to the server, then point the main window at it.
fn boot_and_connect(handle: &AppHandle) -> Result<(), String> {
    let host = std::env::var("DSH_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("DSH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3080);
    let url = format!("http://{host}:{port}");
    let addr = format!("{host}:{port}")
        .parse::<SocketAddr>()
        .map_err(|_| format!("invalid bind address {host}:{port}"))?;

    if !port_open(addr) {
        spawn_dsh(handle, &host, port)?;
        wait_for_port(addr, Duration::from_secs(60))?;
    } else {
        println!("[dsh-desktop] DeepSeek Harness already running at {url}");
    }

    let window = wait_for_window(handle, Duration::from_secs(15))?;
    let parsed = url.parse::<tauri::Url>().map_err(|e| e.to_string())?;
    window.navigate(parsed).map_err(|e| e.to_string())?;
    println!("[dsh-desktop] navigated to {url}");
    Ok(())
}

/// Resolve the `dsh` executable.
///
/// Precedence:
///   1. `DSH_BIN` env var (explicit override)
///   2. `dsh` on `PATH`
///   3. Common per-user install locations (`~/.local/bin`, `~/.npm-global/bin`,
///      any nvm node version's bin)
///   4. The npx cache (`~/.npm/_npx/*/node_modules/.bin/dsh`) — where `dsh`
///      ends up when run via `npx @deepseek-ai/dsh`
///
/// Desktop-launched apps do not inherit the shell's PATH (nvm/npx dirs are
/// missing), so the npx-cache scan is what makes "click the icon" work out of
/// the box.
fn resolve_dsh_bin() -> Option<PathBuf> {
    // 1) explicit override
    if let Ok(bin) = std::env::var("DSH_BIN") {
        let p = PathBuf::from(bin);
        if is_executable(&p) {
            return Some(p);
        }
    }
    // 2) PATH
    if let Some(p) = find_on_path("dsh") {
        return Some(p);
    }
    // 3) common per-user install locations
    let home = std::env::var("HOME").ok()?;
    let mut candidates = vec![
        PathBuf::from(&home).join(".local/bin/dsh"),
        PathBuf::from(&home).join(".npm-global/bin/dsh"),
    ];
    let nvm = PathBuf::from(&home).join(".nvm/versions/node");
    if let Ok(entries) = std::fs::read_dir(&nvm) {
        let mut versions: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        versions.sort();
        for v in versions {
            candidates.push(v.join("bin/dsh"));
        }
    }
    for c in candidates {
        if is_executable(&c) {
            return Some(c);
        }
    }
    // 4) npx cache: ~/.npm/_npx/<hash>/node_modules/.bin/dsh
    let npx = PathBuf::from(&home).join(".npm/_npx");
    if let Ok(entries) = std::fs::read_dir(&npx) {
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for d in dirs {
            let bin = d.join("node_modules/.bin/dsh");
            if is_executable(&bin) {
                return Some(bin);
            }
        }
    }
    None
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_file() && p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// Build a PATH for the spawned `dsh` that puts modern per-user tooling ahead
/// of the system dirs.
///
/// Desktop-launched apps get a minimal PATH (e.g. `/usr/bin`) that only has the
/// old system Node, which crashes `dsh` (`node:util.parseEnv` needs Node >= 21).
/// Prepend nvm's node bins, `~/.local/bin`, `~/.npm-global/bin`, and the
/// resolved `dsh`'s own bin dir so `#!/usr/bin/env node` and any node
/// subprocesses `dsh` spawns resolve to the right interpreter.
#[cfg(not(windows))]
fn augmented_path(bin: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(parent) = Path::new(bin).parent() {
        dirs.push(parent.to_path_buf());
    }
    dirs.push(PathBuf::from(&home).join(".local/bin"));
    dirs.push(PathBuf::from(&home).join(".npm-global/bin"));
    // nvm node versions, newest first
    let nvm_root = PathBuf::from(&home).join(".nvm");
    if let Ok(entries) = std::fs::read_dir(nvm_root.join("versions/node")) {
        let mut versions: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        versions.sort();
        versions.reverse();
        for v in versions {
            dirs.push(v.join("bin"));
        }
    }
    let current = nvm_root.join("current/bin");
    if current.is_dir() {
        dirs.push(current);
    }
    // keep the rest of the existing PATH after ours
    if let Some(existing) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&existing) {
            dirs.push(d.into());
        }
    }
    std::env::join_paths(&dirs).ok().map(|p| p.to_string_lossy().into_owned())
}

/// Spawn `dsh web --host <host> --port <port>` and remember the child.
fn spawn_dsh(handle: &AppHandle, host: &str, port: u16) -> Result<(), String> {
    #[cfg(windows)]
    let bin = std::env::var("DSH_BIN").unwrap_or_else(|_| "dsh".into());
    #[cfg(not(windows))]
    let bin = resolve_dsh_bin()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "dsh".into());

    #[cfg(windows)]
    let mut cmd = {
        // std::process cannot resolve .cmd shims; go through cmd.exe.
        let mut c = Command::new("cmd");
        c.args(["/C", &format!("{bin} web --host {host} --port {port}")]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new(&bin);
        c.args(["web", "--host", host, "--port", &port.to_string()]);
        if let Some(path) = augmented_path(&bin) {
            c.env("PATH", path);
        }
        c
    };

    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let child = cmd.spawn().map_err(|e| {
        format!(
            "failed to spawn `{bin} web --host {host} --port {port}`: {e}\n\
             Make sure `dsh` is available (npm i -g @deepseek-ai/dsh, or run it\n\
             once via `npx @deepseek-ai/dsh` so it lands in the npx cache),\n\
             or set DSH_BIN to the executable path, or start `dsh web` yourself."
        )
    })?;
    if let Some(state) = handle.try_state::<DshChild>() {
        state.0.lock().unwrap().replace(child);
    }
    Ok(())
}

fn port_open(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

fn wait_for_port(addr: SocketAddr, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if port_open(addr) {
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(format!("timed out waiting for {addr} to accept connections"));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// The config-declared window may be created slightly after `setup` runs;
/// poll for it.
fn wait_for_window(
    handle: &AppHandle,
    timeout: Duration,
) -> Result<tauri::WebviewWindow, String> {
    let start = Instant::now();
    loop {
        if let Some(win) = handle.get_webview_window("main") {
            return Ok(win);
        }
        if start.elapsed() > timeout {
            return Err("main window never appeared".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Best-effort: show the failure message inside the window via a data: URL.
fn show_error(handle: &AppHandle, message: &str) {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br>");
    let body = format!(
        "<h2>DeepSeek Harness failed to start</h2><pre style=\"white-space:pre-wrap\">{escaped}</pre>"
    );
    let encoded = percent_encode(&body);
    let data = format!("data:text/html;charset=utf-8,{encoded}");
    if let Ok(url) = data.parse::<tauri::Url>() {
        if let Some(win) = handle.get_webview_window("main") {
            let _ = win.navigate(url);
        }
    }
}

/// Minimal RFC 3986 percent-encoding for building a data: URL.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b' ' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
