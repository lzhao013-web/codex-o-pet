#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cargo install --path "$ROOT" --force
command -v codex-o-pet-bridge
