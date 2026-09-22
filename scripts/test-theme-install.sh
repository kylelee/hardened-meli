#!/bin/bash
# SPDX-License-Identifier: EUPL-1.2
#
# Repeatably verify the sample themes in themes/.
#
# Simulates what a user installing a theme does — copy the TOML files into
# $XDG_CONFIG_HOME/meli/themes/ — and then checks what meli's `:toggle theme`
# picker would discover in that directory (mirroring conf::get_user_themes):
# every *.toml must parse and expose at least one non-reserved
# [terminal.themes.<name>] table.
#
# Usage: scripts/test-theme-install.sh
# Requires: bash, python3 (>= 3.11), coreutils.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SAMPLES_DIR="$ROOT/themes"

if [ ! -d "$SAMPLES_DIR" ]; then
    echo "error: $SAMPLES_DIR does not exist" >&2
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Simulated user install: ~/.config equivalent under a throwaway directory.
XDG_CONFIG_HOME="$TMP/config"
mkdir -p "$XDG_CONFIG_HOME/meli/themes"
cp "$SAMPLES_DIR"/*.toml "$XDG_CONFIG_HOME/meli/themes/"
echo "installed $(ls "$XDG_CONFIG_HOME/meli/themes"/*.toml | wc -l) theme files into $XDG_CONFIG_HOME/meli/themes/"

XDG_CONFIG_HOME="$XDG_CONFIG_HOME" python3 - "$XDG_CONFIG_HOME/meli/themes" <<'PY'
import sys
import tomllib
from pathlib import Path

themes_dir = Path(sys.argv[1])
assert themes_dir.is_dir(), f"{themes_dir}: not a directory"
paths = sorted(themes_dir.glob("*.toml"))
assert paths, f"{themes_dir}: no *.toml theme files"

total = 0
for path in paths:
    with path.open("rb") as f:
        value = tomllib.load(f)
    themes = (
        value.get("terminal", {}).get("themes")
        if isinstance(value.get("terminal", {}), dict)
        else None
    )
    assert isinstance(themes, dict), f"{path.name}: no [terminal.themes] table"
    assert themes, f"{path.name}: [terminal.themes] is empty"
    for name in themes:
        assert name not in ("light", "dark"), (
            f"{path.name}: theme name `{name}` is reserved"
        )
    total += len(themes)
    print(f"{path.name}: {', '.join(sorted(themes))}")

print(f"OK: {len(paths)} files, {total} themes discoverable by `:toggle theme`")
PY
