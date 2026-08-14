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

/// Spawn `dsh web --host <host> --port <port>` and remember the child.
fn spawn_dsh(handle: &AppHandle, host: &str, port: u16) -> Result<(), String> {
    let bin = std::env::var("DSH_BIN").unwrap_or_else(|_| "dsh".into());

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
        c
    };

    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let child = cmd.spawn().map_err(|e| {
        format!(
            "failed to spawn `{bin} web --host {host} --port {port}`: {e}\n\
             Make sure `dsh` is installed and on PATH (npm i -g @deepseek-ai/dsh),\n\
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
