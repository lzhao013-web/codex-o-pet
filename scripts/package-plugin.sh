#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <bridge-artifacts-dir> <publication-dir> <version>" >&2
    exit 2
fi

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ARTIFACTS=$(CDPATH= cd -- "$1" && pwd)
OUTPUT=$2
VERSION=$3

case "$OUTPUT" in
    '' | / | .)
        echo "unsafe publication directory: $OUTPUT" >&2
        exit 2
        ;;
esac

case "$VERSION" in
    '' | *[!0-9A-Za-z.+-]*)
        echo "invalid plugin version: $VERSION" >&2
        exit 2
        ;;
esac

rm -rf "$OUTPUT"
mkdir -p "$OUTPUT/plugin/bin" "$OUTPUT/.agents/plugins"
cp -R "$ROOT/plugin/." "$OUTPUT/plugin/"

copy_bridge() {
    source=$1
    destination=$2
    if [ ! -f "$source" ]; then
        echo "missing bridge artifact: $source" >&2
        exit 1
    fi
    cp "$source" "$OUTPUT/plugin/bin/$destination"
}

copy_bridge "$ARTIFACTS/bridge-windows-x86_64/codex-o-pet-bridge.exe" \
    codex-o-pet-bridge.exe
copy_bridge "$ARTIFACTS/bridge-linux-x86_64/codex-o-pet-bridge" \
    codex-o-pet-bridge-linux-x86_64
copy_bridge "$ARTIFACTS/bridge-linux-aarch64/codex-o-pet-bridge" \
    codex-o-pet-bridge-linux-aarch64
copy_bridge "$ARTIFACTS/bridge-macos-x86_64/codex-o-pet-bridge" \
    codex-o-pet-bridge-macos-x86_64
copy_bridge "$ARTIFACTS/bridge-macos-aarch64/codex-o-pet-bridge" \
    codex-o-pet-bridge-macos-aarch64
cp "$ROOT/packaging/codex-o-pet-bridge" \
    "$OUTPUT/plugin/bin/codex-o-pet-bridge"
chmod 755 "$OUTPUT/plugin/bin/"*

python3 - "$OUTPUT/plugin" "$VERSION" <<'PY'
import json
import pathlib
import sys

plugin_root = pathlib.Path(sys.argv[1])
version = sys.argv[2]

manifest_path = plugin_root / ".codex-plugin" / "plugin.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
manifest["version"] = version
manifest_path.write_text(
    json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)

mcp_path = plugin_root / "mcp.json"
mcp = json.loads(mcp_path.read_text(encoding="utf-8"))
server = mcp["mcpServers"]["codex-o-pet"]
server["command"] = "./bin/codex-o-pet-bridge"
server["cwd"] = "."
mcp_path.write_text(
    json.dumps(mcp, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
PY

cat > "$OUTPUT/.agents/plugins/marketplace.json" <<EOF
{
  "name": "codex-o-pet",
  "interface": {
    "displayName": "Codex o-pet"
  },
  "plugins": [
    {
      "name": "codex-o-pet",
      "source": {
        "source": "local",
        "path": "./plugin"
      },
      "policy": {
        "installation": "AVAILABLE",
        "authentication": "ON_INSTALL"
      },
      "category": "Developer Tools"
    }
  ]
}
EOF

cat > "$OUTPUT/README.md" <<EOF
# codex-o-pet plugin $VERSION

This branch contains the packaged Codex plugin. It includes Bridge binaries for Windows x86_64, Linux x86_64/ARM64, and macOS Intel/Apple Silicon.

Install it with:

\`\`\`bash
codex plugin marketplace add lzhao013-web/codex-o-pet --ref plugin-dist
codex plugin add codex-o-pet@codex-o-pet
\`\`\`

Start [o-pet](https://github.com/Orion-zhen/o-pet) before opening a new Codex session.
EOF

python3 - "$OUTPUT" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
manifest = json.loads((root / "plugin/.codex-plugin/plugin.json").read_text())
mcp = json.loads((root / "plugin/mcp.json").read_text())
marketplace = json.loads((root / ".agents/plugins/marketplace.json").read_text())

assert manifest["name"] == "codex-o-pet"
assert mcp["mcpServers"]["codex-o-pet"]["command"] == "./bin/codex-o-pet-bridge"
assert marketplace["plugins"][0]["source"]["path"] == "./plugin"

expected = {
    "codex-o-pet-bridge",
    "codex-o-pet-bridge.exe",
    "codex-o-pet-bridge-linux-x86_64",
    "codex-o-pet-bridge-linux-aarch64",
    "codex-o-pet-bridge-macos-x86_64",
    "codex-o-pet-bridge-macos-aarch64",
}
actual = {path.name for path in (root / "plugin/bin").iterdir()}
assert actual == expected, (actual, expected)
PY
