#!/usr/bin/env bash
# Install the `loopy` binary so you can run `loopy` from anywhere
# instead of `cargo run --`.
set -euo pipefail

PKG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"

echo "Building release binary..."
cd "$PKG_DIR"
cargo build --release

mkdir -p "$BIN_DIR"
ln -sf "$PKG_DIR/target/release/loopy" "$BIN_DIR/loopy"

echo ""
echo "✅ Installed: $BIN_DIR/loopy -> $PKG_DIR/target/release/loopy"

if echo "$PATH" | tr ':' '\n' | grep -qx "$BIN_DIR"; then
  echo "   ~/.local/bin is on your PATH — run 'loopy start' from anywhere."
else
  echo "   ⚠️  Add ~/.local/bin to your PATH:"
  echo "      echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
fi
echo ""
echo "To update after code changes, re-run this script (the symlink points"
echo "at the release binary, so 'cargo build --release' alone also refreshes it)."
