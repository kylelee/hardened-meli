#!/bin/sh
# SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
#
# Build meli (release) and install the executable
# (meli) into BIN_DIR (default: ${HOME}/.local/bin).

set -e

cd "$(dirname "$0")/.."

BIN_DIR="${BIN_DIR:-${HOME}/.local/bin}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"

echo "=== cargo build --release ==="
cargo build --release

mkdir -p "${BIN_DIR}"
for bin in meli; do
    if [ ! -f "${TARGET_DIR}/release/${bin}" ]; then
        echo "ERROR: ${TARGET_DIR}/release/${bin} not found" >&2
        exit 1
    fi
    cp "${TARGET_DIR}/release/${bin}" "${BIN_DIR}/${bin}"
    chmod 755 "${BIN_DIR}/${bin}"
    echo " - installed ${BIN_DIR}/${bin}"
done

case ":${PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) echo "WARNING: ${BIN_DIR} is not in your PATH; consider adding it." >&2 ;;
esac
