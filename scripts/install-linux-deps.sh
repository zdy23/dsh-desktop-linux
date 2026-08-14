#!/usr/bin/env bash
# Install the system dependencies needed to build & run a Tauri v2 app on
# Debian/Ubuntu, plus the Rust toolchain if missing.
#
#   ./scripts/install-linux-deps.sh
#
# Requires sudo (or root). Debian 12 / Ubuntu 22.04+ recommended so that
# webkit2gtk-4.1 is available.
set -euo pipefail

echo "==> Installing system packages (webkit2gtk-4.1, gtk3, build tools)..."
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev

if ! command -v cargo >/dev/null 2>&1; then
  echo "==> Installing Rust toolchain via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
else
  echo "==> cargo already installed: $(cargo --version)"
fi

echo "==> Verifying..."
cargo --version
pkg-config --modversion webkit2gtk-4.1
echo "==> Done. Now run: npm install && npm run dev"
