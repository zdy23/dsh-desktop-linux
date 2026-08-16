# DeepSeek Harness Desktop (Tauri)

Wraps the DeepSeek Harness web GUI (served by `dsh web` at `http://127.0.0.1:3080`) into a native desktop application.

## How it works

When the Tauri shell starts:

1. If nothing is listening on `127.0.0.1:3080`, it runs `dsh web --host 127.0.0.1 --port <port>` (default 3080) to boot the backend;
2. It polls the port until the HTTP server is ready (60 s timeout);
3. It navigates the app window to `http://127.0.0.1:<port>`;
4. When the window closes, it kills the `dsh` child process it spawned (if the server was already started externally, it is left alone).

A built-in splash page is shown briefly before the window navigates to the GUI.

### Clicking the icon starts `dsh` automatically

No need to start `dsh web` in a terminal first — the app finds the `dsh` executable
by itself. Desktop-launched apps don't inherit your shell's `PATH`, so the lookup
goes in this order:

1. `DSH_BIN` env var (explicit override);
2. `dsh` on `PATH`;
3. Common per-user installs: `~/.local/bin/dsh`, `~/.npm-global/bin/dsh`, any
   `~/.nvm/versions/node/*/bin/dsh`;
4. The npx cache: `~/.npm/_npx/*/node_modules/.bin/dsh` (where `dsh` lives when
   you run it via `npx @deepseek-ai/dsh`).

So the typical setup — `dsh` only ever run through `npx` — just works from the
app icon.

## Prerequisites

- Linux (Debian 12 / Ubuntu 22.04+, or any distro where webkit2gtk-4.1 is installable)
- Rust toolchain (`cargo`) — the install script sets it up if missing
- Node.js 18+ (for the Tauri CLI and `dsh`)
- The `dsh` executable on `PATH`: `npm i -g @deepseek-ai/dsh` (or `npx @deepseek-ai/dsh`)

## Building

```bash
# 1) System dependencies + Rust (needs sudo; run once)
./scripts/install-linux-deps.sh

# 2) Install the Tauri CLI (npm wrapper scripts)
npm install

# 3) Dev mode: hot-start, the window connects straight to the local GUI
npm run dev

# 4) Build distributables (.deb and AppImage)
npm run build
# or individually: npm run build:deb / npm run build:appimage
```

Artifacts live in `src-tauri/target/release/bundle/`:

- `deb/` — Debian/Ubuntu package, named `DeepSeek Harness_0.1.0_amd64.deb`. Note the **space** in the filename: quote the path or `dpkg`/`apt` will split it into two arguments and fail with `cannot access archive '.../DeepSeek'`:

  ```bash
  sudo apt install "./src-tauri/target/release/bundle/deb/DeepSeek Harness_0.1.0_amd64.deb"
  # or
  sudo dpkg -i "src-tauri/target/release/bundle/deb/DeepSeek Harness_0.1.0_amd64.deb"
  ```

  (Tab-completion escapes the space for you automatically.)
- `appimage/` — portable AppImage (`chmod +x`, then run)

## Configuration (environment variables)

| Variable   | Default     | Description                                        |
| ---------- | ----------- | -------------------------------------------------- |
| `DSH_BIN`  | `dsh`       | Path to the `dsh` executable (use the `.cmd` path on Windows) |
| `DSH_HOST` | `127.0.0.1` | Bind address of the backend                        |
| `DSH_PORT` | `3080`      | Backend port; an already-listening service is reused as-is |

## Daily use

The desktop app is just a shell — it spawns `dsh web` for you. You can also start `dsh web --port 3080` manually first and then open the app: it detects the ready port and connects directly without starting a second instance.

## Directory layout

```
src-tauri/
  src/main.rs        # Rust logic: spawn dsh web, wait for port, navigate window, cleanup on exit
  tauri.conf.json    # Window and bundling config (deb / appimage)
  capabilities/      # Tauri v2 permissions (minimal default set)
  icons/             # Icons generated from the GUI favicon
ui/index.html        # Startup splash page (frontendDist)
scripts/install-linux-deps.sh  # System dependency + Rust installer
```

## Other platforms

- **Windows**: relax the `bundle.targets` restriction in `tauri.conf.json` (or set `["msi","nsis"]`), make sure `dsh` is on `PATH` (the `.cmd` shim from a global npm install works too — the code goes through `cmd /C`). `icon.ico` is already included.
- **macOS**: needs `icon.icns`; generate the full platform icon set with `npm run tauri icon` from `icons/icon.png`.
