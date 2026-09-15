#!/bin/bash
# SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
#
# Private-CSI input-stall watchdog PTY regression (drop-vendored-crossterm
# plan, task T5).
#
# P1 — healthy session: >=10s of idle input; the input watchdog must never
#      inject a DA1 query (zero false injections).
# P2 — stall self-heal: tmux injects the private-mode report
#      `CSI ? 2026;2$y`, which the upstream crossterm 0.29 parser buffers
#      indefinitely while swallowing every byte that follows. The watchdog
#      must inject its DA1 query (ESC[c) on its own, after which the next
#      key press is processed again. Keys swallowed during the stall are
#      discarded with the buffer flush and are NOT recoverable by design;
#      what is asserted is the recovery of subsequent input.
#
# Binary prerequisite: meli's input loop must hand crossterm a non-blocking
# fd 0 (stdin re-opened through /dev/tty O_RDWR|O_NONBLOCK). crossterm
# 0.29's mio source drains the tty in a loop that only ends on WouldBlock;
# with a plain blocking stdin the input thread parks in blocking read(2)
# *inside* event::poll(ZERO) at the first incomplete sequence, never
# returns to meli's poll(2) loop, and the watchdog can observe nothing -
# the stall becomes permanent. P2 fails in exactly that way (no DA1
# injection within its budget) if that fd handling regresses.
#
# Environment hazards pinned down by this script:
# - Theme: `dark` plus a [terminal.themes.dark] override pins the compact
#   listing cursor row to SGR 48;5;24 (cursor rows use the
#   even/odd_highlighted keys, which default to 48;5;240; the explicit
#   theme also shields against OSC 11 palette replies flipping light/dark).
# - NO_COLOR: crossterm drops every color SGR when it is set, so it is
#   cleared for the pane.
#
# Skips (exit 0) when tmux is unavailable.

set -euo pipefail

if ! command -v tmux >/dev/null 2>&1; then
    echo "SKIP: tmux not available; private-CSI watchdog PTY regression requires tmux"
    exit 0
fi

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)
BIN="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/debug/meli"

WORK=$(mktemp -d)
KEEP_WORK=0
SESS="meliwd-$$-$RANDOM"
TMUX=(tmux -L "$SESS")
SOCKET="${TMUX_TMPDIR:-/tmp}/tmux-$(id -u)/$SESS"

cleanup() {
    "${TMUX[@]}" kill-session -t "$SESS" 2>/dev/null || true
    # tmux 3.7 leaves the stale socket file behind once the dedicated
    # server (this session was its only one) exits; remove it.
    rm -f "$SOCKET" 2>/dev/null || true
    if [ "$KEEP_WORK" -eq 1 ]; then
        echo "evidence kept in: $WORK"
    else
        rm -rf "$WORK"
    fi
}
trap cleanup EXIT

fail() {
    KEEP_WORK=1
    "${TMUX[@]}" capture-pane -e -p -t "$SESS" >"$WORK/capture-fail.txt" 2>/dev/null || true
    echo "FAIL: $*"
    exit 1
}

# True while the pane process (meli) is alive.
pane_alive() {
    local dead
    dead=$("${TMUX[@]}" display-message -p -t "$SESS" '#{pane_dead}' 2>/dev/null) || return 1
    [ "$dead" = "0" ]
}

# Minimum 1-based capture line carrying the cursor-row SGR 48;5;24, not
# matching longer codes such as 48;5;240.
sel_row() {
    grep -anE '48;5;24([^0-9]|$)' "$1" | head -n 1 | cut -d: -f1
}

cd "$REPO_ROOT"
if ! cargo build >"$WORK/build.log" 2>&1; then
    tail -n 20 "$WORK/build.log" || true
    fail "cargo build failed"
fi

# Self-contained maildir account: five RFC 822 messages.
MAILDIR="$WORK/maildir"
mkdir -p "$MAILDIR/cur" "$MAILDIR/new" "$MAILDIR/tmp" \
    "$WORK/xdg/config" "$WORK/xdg/cache" "$WORK/xdg/data" "$WORK/xdg/state" \
    "$WORK/home"
for i in 1 2 3 4 5; do
    printf 'From: Sender One <sender%d@example.invalid>\nTo: Recipient <rcpt@example.invalid>\nSubject: WDQMAIL %d\nDate: Wed, 16 Sep 2026 10:0%d:00 +0300\nMessage-ID: <wdq-%d@example.invalid>\n\nBody of mail %d.\n' \
        "$i" "$i" "$i" "$i" "$i" >"$MAILDIR/new/170000000$i.Mwdq$i.Q$i"
done

# Dark theme pinned; cursor rows forced to SGR bg 24 so the marker below is
# deterministic. use_color is forced on (NO_COLOR in the environment would
# otherwise strip every color SGR; the pane env clears it as well).
cat >"$WORK/config.toml" <<EOF
[accounts.wd]
root_mailbox = "$MAILDIR"
format = "maildir"
send_mail = 'false'
identity = "wd@example.invalid"
search_backend = "none"

[terminal]
theme = "dark"
use_color = true

[terminal.themes.dark]
"mail.listing.compact.even_highlighted" = { bg = "24" }
"mail.listing.compact.odd_highlighted" = { bg = "24" }

[listing]
index_style = "compact"
EOF

"${TMUX[@]}" new-session -d -x 120 -y 30 -s "$SESS" \
    "env -u NO_COLOR HOME='$WORK/home' XDG_CONFIG_HOME='$WORK/xdg/config' XDG_CACHE_HOME='$WORK/xdg/cache' XDG_DATA_HOME='$WORK/xdg/data' XDG_STATE_HOME='$WORK/xdg/state' MELI_CONFIG='$WORK/config.toml' MELI_DEBUG_STDERR=yes '$BIN' 2>'$WORK/stderr.log'"

# Bounded wait for the listing to render (a message subject in the pane).
deadline=$((SECONDS + 10))
while ! "${TMUX[@]}" capture-pane -p -t "$SESS" 2>/dev/null | grep -q WDQMAIL; do
    pane_alive || fail "setup: meli pane exited before rendering (stderr: $WORK/stderr.log)"
    if [ "$SECONDS" -ge "$deadline" ]; then
        fail "setup: listing did not render within 10s (stderr: $WORK/stderr.log)"
    fi
    sleep 0.2
done

# P1: >=10s healthy idle; the watchdog must stay silent.
deadline=$((SECONDS + 10))
while [ "$SECONDS" -lt "$deadline" ]; do
    pane_alive || fail "P1: meli pane exited during healthy-idle window (stderr: $WORK/stderr.log)"
    sleep 0.2
done
echo "PASS P1.a pane stayed alive through >=10s healthy idle"

if grep -aq "input watchdog" "$WORK/stderr.log"; then
    grep -a "input watchdog" "$WORK/stderr.log" >"$WORK/stderr-excerpt.txt" || true
    fail "P1: watchdog activity during healthy idle (evidence: $WORK/stderr-excerpt.txt)"
fi
echo "PASS P1.b zero 'input watchdog' lines in stderr over healthy idle (evidence: $WORK/stderr.log)"

# P2: stall, watchdog self-heal, subsequent-key recovery.
# The cursor row must already be visible and marked.
deadline=$((SECONDS + 5))
"${TMUX[@]}" capture-pane -e -p -t "$SESS" >"$WORK/capture-pre.txt"
while :; do
    pre=$(sel_row "$WORK/capture-pre.txt" || true)
    if [ -n "$pre" ]; then
        break
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
        fail "P2: no listing cursor row (SGR 48;5;24) in pre-injection capture (evidence: $WORK/capture-pre.txt)"
    fi
    sleep 0.2
    "${TMUX[@]}" capture-pane -e -p -t "$SESS" >"$WORK/capture-pre.txt"
done

# Inject `CSI ? 2026;2$y` raw bytes; send no keys afterwards until the
# watchdog has injected - everything sent before the flush is swallowed.
"${TMUX[@]}" send-keys -H -t "$SESS" 1b 5b 3f 32 30 32 36 3b 32 24 79

deadline=$((SECONDS + 10))
while ! grep -aq "input watchdog: injecting DA1" "$WORK/stderr.log"; do
    pane_alive || fail "P2: meli pane exited while input was stalled (stderr: $WORK/stderr.log)"
    if [ "$SECONDS" -ge "$deadline" ]; then
        fail "P2: watchdog did not inject DA1 within 10s of private-CSI stall (evidence: $WORK/stderr.log)"
    fi
    sleep 0.2
done
grep -a "input watchdog" "$WORK/stderr.log" >"$WORK/stderr-excerpt.txt"
echo "PASS P2.a watchdog injected DA1 query after stall (evidence: $WORK/stderr-excerpt.txt)"

# Post-recovery key: the selection must move down from the pre-stall row.
"${TMUX[@]}" send-keys -t "$SESS" Down
deadline=$((SECONDS + 5))
post=""
while :; do
    "${TMUX[@]}" capture-pane -e -p -t "$SESS" >"$WORK/capture-post.txt" 2>/dev/null ||
        fail "P2: pane exited after watchdog recovery"
    post=$(sel_row "$WORK/capture-post.txt" || true)
    if [ -n "$post" ] && [ "$post" -gt "$pre" ]; then
        break
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
        fail "P2: selection did not move after watchdog recovery (pre=$pre post=$post; evidence: $WORK/capture-pre.txt $WORK/capture-post.txt)"
    fi
    sleep 0.2
done
echo "PASS P2.b selection recovered: cursor row pre=$pre post=$post (evidence: $WORK/capture-pre.txt, $WORK/capture-post.txt)"

echo "private-CSI watchdog PTY regression: ALL PASS"
echo "evidence dir: $WORK (kept on failure; removed on success)"
