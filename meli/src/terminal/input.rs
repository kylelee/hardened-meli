//
// meli
//
// Copyright 2017 - Manos Pitsidianakis
//
// This file is part of meli.
//
// meli is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// meli is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with meli. If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

use std::{
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::fs::OpenOptionsExt,
    },
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use crate::terminal::keys::Key;
use crate::terminal::ratatui_bridge::{encode_key, BridgeEvent};
use crossbeam::channel::{Receiver, RecvTimeoutError};
use crossterm::event;
use melib::log;
use nix::{
    errno::Errno,
    poll::{poll, PollFd, PollFlags, PollTimeout},
};

/*
 * The input loop itself now parses with crossterm (see `get_events`); the
 * byte tables `encode_key` must reproduce are pinned by the ratatui_bridge
 * tests.
 */

#[derive(Debug, Default)]
/// Main process sends commands to the input thread.
pub enum InputCommand {
    #[default]
    /// Exit thread
    Kill,
}

/// Upper bound between checks of the command pipe in the input loop.
///
/// The blocking wait of the loop is meli's `poll(2)` over `{tty,
/// command-pipe}` (see [`get_events`]). Terminal input wakes it
/// immediately; resizes do not: crossterm watches `SIGWINCH` with its own
/// internal waker, which is not part of meli's poll set, so a resize is
/// only noticed the next time the loop consults crossterm - at most this
/// interval later, where the previous blocking `event::poll` delivered it
/// immediately. A kill command (`InputCommand::Kill` plus its pipe byte)
/// is likewise bounded by this interval.
const KILL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How long [`take_command`] waits for the command message to land after
/// the pipe byte was seen readable; mirrors the deadline of the `pselect`
/// dance this loop replaced.
const COMMAND_RECV_TIMEOUT: Duration = Duration::from_secs(2);

/// Process-wide kill switch for watchdog wiring.
///
/// Flipped when no tty fd can serve the watchdog: stdin is not a terminal
/// and `/dev/tty` cannot be opened, or the tty hung up. It lives outside
/// [`watchdog`] because the state machine's own `State::Disabled` is only
/// reachable through its decisions. Like the rest of the watchdog state
/// it is process-global: `$EDITOR` round-trips re-spawn the input thread
/// through `InputHandler::restore`, and a halted watchdog must not be
/// revived by that any more than a `Disabled` one.
static WATCHDOG_HALTED: AtomicBool = AtomicBool::new(false);

/// Whether [`halt_watchdog`] has fired for this process.
fn watchdog_halted() -> bool {
    WATCHDOG_HALTED.load(Ordering::Relaxed)
}

/// Halts watchdog wiring for the rest of the process.
fn halt_watchdog() {
    WATCHDOG_HALTED.store(true, Ordering::Relaxed);
}

/// A stashed fd 0 plus the identity of the [`FdSwap`] that stashed it.
struct SavedStdin {
    /// Which generation of stdin swap owns this stash; a dropped guard
    /// only restores its own generation (see [`SAVED_STDIN`]).
    generation: u64,
    fd: OwnedFd,
}

/// The original fd 0 description, stashed while an input thread holds
/// fd 0 swapped to a nonblocking tty (see [`FdSwap`]).
///
/// Ownership lives here rather than in the guard so that either the
/// guard's drop *or* a child spawn can restore it, whichever comes
/// first - and the entry carries the swap's generation so the two can
/// tell each other apart around the `$EDITOR` round trip: the main
/// thread kills input thread T1, spawns the child, and re-spawns the
/// input thread as T2, all possibly before T1 has exited. T2 installs
/// its own swap and stashes it here; when T1 finally drops, it must
/// restore *only its own* entry - stealing T2's would silently point fd
/// 0 back at the blocking description while T2 believes it swapped,
/// resurrecting the crossterm wedge. [`restore_stdin_for_child_spawn`]
/// (main thread) restores unconditionally; a guard's drop restores
/// through [`restore_stdin_if_current`] and is a no-op on any mismatch.
static SAVED_STDIN: Mutex<Option<SavedStdin>> = Mutex::new(None);

/// Identity of the current stdin swap; bumped by every [`FdSwap`]
/// install so guards only ever restore what they installed.
static STDIN_SWAP_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The single description every fd 0 swap installs, kept for the rest of
/// the process.
///
/// crossterm's process-global event reader registers the fd 0 open file
/// description it finds on first use with `mio`/`epoll`. Every `$EDITOR`
/// round trip replaces fd 0 via `dup2`, which would close the previously
/// registered description; the kernel then silently drops its epoll
/// registration, `event::poll` answers "no event" forever while meli's own
/// `poll(2)` keeps seeing the tty readable (input dies, the loop spins),
/// and even `watchdog`'s DA1 injection cannot help because the reply can
/// no longer be read. Keeping this description alive keeps the
/// registration valid: readiness is a property of the tty, so the stale
/// entry still fires even though fd 0 was re-pointed.
///
/// Only the *first* swapped-in description is retained — the event reader
/// is created once, on the input thread's first `poll`, so the first swap
/// is the registered one. Every later swap `dup2`s this same description
/// onto fd 0 and drops its own freshly opened `/dev/tty` instead of
/// retaining it: the previous `Vec` grew by one descriptor (and one
/// `open(2)`) per `$EDITOR` round trip for the whole process lifetime.
static SWAP_KEEPALIVE: Mutex<Option<OwnedFd>> = Mutex::new(None);

/// Restores fd 0 to the descriptor the process was started with, if an
/// input thread currently holds it swapped to a nonblocking tty.
///
/// Called before every interactive child spawn (`$EDITOR` et al.,
/// `crate::state::State::execute_command`) so the child cannot inherit
/// the swapped-in nonblocking descriptor. Restores unconditionally, of
/// any generation - after this, a still-running thread's guard drop
/// finds no (or a foreign) entry and naturally does nothing.
pub(crate) fn restore_stdin_for_child_spawn() {
    if let Some(saved) = SAVED_STDIN
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        if let Err(err) = nix::unistd::dup2_stdin(&saved.fd) {
            log::trace!("get_events: could not restore stdin: {err}");
        }
    }
}

/// Like [`restore_stdin_for_child_spawn`], but only touches the stash if
/// it still belongs to `generation`: an older guard's drop must not
/// steal a newer swap's stash.
fn restore_stdin_if_current(generation: u64) {
    let mut stash = SAVED_STDIN.lock().unwrap_or_else(|err| err.into_inner());
    if stash
        .as_ref()
        .is_some_and(|saved| saved.generation == generation)
    {
        if let Some(saved) = stash.take() {
            if let Err(err) = nix::unistd::dup2_stdin(&saved.fd) {
                log::trace!("get_events: could not restore stdin: {err}");
            }
        }
    }
}

/// Feeds one observation to the process-global watchdog and answers its
/// decision; a no-op once [`halt_watchdog`] has fired.
fn observe_watchdog(observation: watchdog::Observation) -> watchdog::Decision {
    if watchdog_halted() {
        return watchdog::Decision::NoAction;
    }
    watchdog::global_watchdog()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .observe(observation)
}

/// Whether the process-global watchdog sits in its terminal `Disabled`
/// state; consulted right after an injection, where `true` means the
/// [`watchdog::MAX_FAILED_INJECTIONS`]th attempt just booked itself.
fn watchdog_disabled() -> bool {
    matches!(
        watchdog::global_watchdog()
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .state(),
        watchdog::State::Disabled
    )
}

/// Services the kill pipe once `poll(2)` saw its byte readable: consumes
/// the byte, then waits - bounded by [`COMMAND_RECV_TIMEOUT`] - for the
/// command message itself.
///
/// `InputHandler::kill` (`crate::state`) writes the pipe byte *before*
/// sending `InputCommand::Kill`, so a readable byte does not mean the
/// message has landed. Receiving non-blockingly inside that window would
/// consume the byte, miss the message, and leave nothing able to wake
/// the loop again: the input thread would never exit, and handing stdin
/// over to `$EDITOR` depends on that exit. The byte is consumed even
/// without a message so the pipe does not stay readable and re-wake the
/// loop.
///
/// Returns `true` for `InputCommand::Kill` - and for a disconnected
/// channel: the pipe write end belongs to the same `InputHandler` that
/// owns the sender, so a disconnect at shutdown leaves the pipe at EOF
/// with `POLLIN` permanently ready, and staying alive could only spin
/// the loop. The previous loop exited in exactly this situation via
/// `unwrap_or_default`. Only a timeout - the byte was real but the
/// message has not landed yet - lets the caller carry on.
fn take_command(rx: &Receiver<InputCommand>, new_command_fd: &OwnedFd) -> bool {
    let mut buf = [0; 2];
    let _ = nix::unistd::read(new_command_fd, &mut buf);
    match rx.recv_timeout(COMMAND_RECV_TIMEOUT) {
        Ok(InputCommand::Kill) => true,
        Err(RecvTimeoutError::Timeout) => false,
        Err(RecvTimeoutError::Disconnected) => true,
    }
}

/// Drains every event crossterm has parsed or buffered without blocking,
/// forwarding keys to `closure` and resizes to `resize`; every delivered
/// event is reported to the watchdog as evidence of a healthy parser.
///
/// Returns `true` once a key event was delivered: the caller then re-checks
/// the command pipe (input thread kill) before waiting again.
fn drain_events(mut closure: impl FnMut((Key, Vec<u8>)), mut resize: impl FnMut(u16, u16)) -> bool {
    'stdin_while: loop {
        match event::poll(Duration::ZERO) {
            Ok(true) => match event::read() {
                Ok(ev) => match BridgeEvent::from(ev) {
                    BridgeEvent::Key(key) => {
                        let bytes = encode_key(&key);
                        log::debug!("input delivered: {key:?} (bytes {bytes:?})");
                        closure((key, bytes));
                        // A delivered event can never answer
                        // `ShouldInject`; the decision is deliberately
                        // dropped.
                        let _ = observe_watchdog(watchdog::Observation::EventDelivered {
                            now: Instant::now(),
                        });
                        return true;
                    }
                    BridgeEvent::Resize(cols, rows) => {
                        log::debug!("input delivered: Resize {cols}x{rows}");
                        resize(cols, rows);
                        let _ = observe_watchdog(watchdog::Observation::EventDelivered {
                            now: Instant::now(),
                        });
                        continue 'stdin_while;
                    }
                    BridgeEvent::Ignored => continue 'stdin_while,
                },
                Err(err) => {
                    log::trace!("get_events: crossterm read error: {err}");
                    return false;
                }
            },
            Ok(false) => return false,
            Err(err) => {
                log::trace!("get_events: crossterm poll error: {err}");
                return false;
            }
        }
    }
}

/// Whether the degraded loop may still ask crossterm for events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CrosstermConsultation {
    /// Never: the tty hung up, so the fd crossterm would read is dead
    /// and its answers are meaningless.
    Never,
    /// Non-blockingly, until the first error: meli could not resolve a
    /// tty fd, but crossterm's own `/dev/tty` fallback may still work,
    /// keeping resizes and events alive.
    UntilFirstError,
}

/// The loop [`get_events`] degrades to when no tty fd can be polled:
/// stdin is not a terminal and `/dev/tty` cannot be opened, or the tty
/// hung up (`POLLHUP`/`POLLERR`, which would re-wake `poll(2)`
/// immediately and spin the loop).
///
/// Its rhythm comes from `poll(2)` over the command pipe alone with a
/// [`KILL_POLL_INTERVAL`] deadline - never from `event::poll`, which
/// without a resolvable tty returns `Err` immediately (spinning the
/// loop) and on a hung-up tty can swallow the error and never answer at
/// all (starving the kill service). Whether crossterm may still be
/// consulted for events depends on the entry ([`CrosstermConsultation`]):
/// after a hang-up it never is; a `/dev/tty` open failure keeps asking
/// non-blockingly - resizes and events stay alive as long as crossterm's
/// own `/dev/tty` fallback works - until the first error, after which
/// this is a pure kill tick. The watchdog is halted for the process
/// before entry: without a tty fd in the poll set the loop can neither
/// observe tty readiness (polling e.g. `stdin` = `/dev/null` would
/// report `POLLIN` forever and busy-spin) nor inject into one.
fn degraded_get_events(
    mut closure: impl FnMut((Key, Vec<u8>)),
    mut resize: impl FnMut(u16, u16),
    rx: &Receiver<InputCommand>,
    new_command_fd: &OwnedFd,
    consultation: CrosstermConsultation,
) {
    let mut consult_crossterm = consultation == CrosstermConsultation::UntilFirstError;
    let poll_timeout = PollTimeout::try_from(KILL_POLL_INTERVAL)
        .expect("KILL_POLL_INTERVAL fits in a poll(2) timeout");
    'degraded_while: loop {
        // The blocking wait and the rhythm of this loop: the command
        // pipe only, at most [`KILL_POLL_INTERVAL`] at a time.
        let mut poll_fds = [PollFd::new(new_command_fd.as_fd(), PollFlags::POLLIN)];
        match poll(&mut poll_fds, poll_timeout) {
            Ok(_n_ready) => {
                // `POLLHUP` counts too: at shutdown the write end is
                // dropped and the read end hangs up (on Linux it reports
                // `POLLHUP`, not `POLLIN`) - `take_command` turns the
                // matching channel disconnect into an exit.
                if poll_fds[0].revents().is_some_and(|ev| {
                    ev.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR)
                }) && take_command(rx, new_command_fd)
                {
                    return;
                }
            }
            Err(Errno::EINTR) => {}
            Err(err) => {
                log::trace!("get_events: poll(2) error: {err}");
                break 'degraded_while;
            }
        }
        if consult_crossterm {
            // Non-blocking only: after the first error crossterm's own
            // fallback is dead too and this is a pure kill tick.
            match event::poll(Duration::ZERO) {
                Ok(true) => {
                    if drain_events(&mut closure, &mut resize) {
                        // Re-check the command pipe (input thread kill)
                        // between events, like the main loop does.
                        continue 'degraded_while;
                    }
                }
                Ok(false) => {}
                Err(err) => {
                    log::trace!(
                        "get_events: crossterm poll error: {err}; \
                         stopping crossterm consultation in degraded loop"
                    );
                    consult_crossterm = false;
                }
            }
        }
    }
}

/// Which descriptor an [`FdSwap`] holds swapped.
#[derive(Debug)]
enum FdSwapTarget {
    /// fd 0, which no `OwnedFd` may claim (`std::io::stdin()` is a
    /// non-owning handle): swapped with `dup2_stdin` and restored through
    /// the process-global [`SAVED_STDIN`], so child spawns can restore it
    /// too.
    Stdin,
    /// A spare, test-owned descriptor: the unit tests drive the
    /// swap/restore/idempotence semantics through high fd numbers
    /// instead of fd 0, so only test code constructs this.
    #[cfg_attr(not(test), allow(dead_code))]
    Owned(OwnedFd),
}

/// Temporarily points a descriptor at a different open file description
/// and restores the original on drop.
///
/// The input thread needs this because crossterm 0.29's internal read
/// loop only exits on `WouldBlock`: with a blocking fd 0 it consumes
/// bytes, produces no event for e.g. an unrecognized private CSI, loops
/// and then parks in `read(2)` forever - wedged inside crossterm, never
/// returning to meli's `poll(2)` loop, where neither the watchdog's
/// suspicious-cycle observation nor its ticks can run. Swapping fd 0 to
/// a freshly opened `O_RDWR | O_NONBLOCK` `/dev/tty` makes that loop see
/// `EAGAIN` and return; the fresh open is an independent description,
/// so fd 1 and fd 2 keep the original blocking one and the main
/// thread's ratatui flush can never `EAGAIN`.
///
/// Restoring is deliberately dual, idempotent and generation-checked:
/// [`restore_stdin_for_child_spawn`] restores unconditionally before any
/// interactive child spawn (`$EDITOR`), killing the race where the child
/// forks while the input thread has not exited yet and would inherit
/// the nonblocking fd 0; the guard's drop restores only the stash of its
/// own swap generation, so an old thread exiting late cannot steal the
/// stash a re-spawned thread already owns (which would silently re-point
/// that thread's fd 0 at the blocking description).
#[derive(Debug)]
struct FdSwap {
    target: FdSwapTarget,
    /// Identity of this swap in [`SAVED_STDIN`] (stdin target only;
    /// the `Owned` target restores locally and never consults the
    /// stash).
    generation: u64,
    saved: Option<OwnedFd>,
}

impl FdSwap {
    /// Points `target`'s descriptor at `replacement`'s open file
    /// description, saving the original for [`FdSwap::restore`]/drop.
    ///
    /// For [`FdSwapTarget::Stdin`] a stale swap is *really* restored
    /// first (`dup2` back onto fd 0, then closed): a second input
    /// thread must not overwrite - and thereby drop - a live stash,
    /// and its own `dup(0)` must capture the original description, not
    /// the previous swap's. Returns `None` - with the target untouched
    /// - if the `dup`/`dup2` bookkeeping fails.
    fn install(target: FdSwapTarget, replacement: OwnedFd) -> Option<Self> {
        let stdin_target = matches!(target, FdSwapTarget::Stdin);
        if stdin_target {
            restore_stdin_for_child_spawn();
        }
        // See [`SWAP_KEEPALIVE`]: every stdin swap installs the same
        // process-lifetime description, so the caller's freshly opened
        // `replacement` is only adopted on the first swap and is closed on
        // every later one. The guard is held for the whole call so the
        // borrowed descriptor stays alive across the `dup2`.
        let mut keepalive;
        let replacement: &OwnedFd = if stdin_target {
            keepalive = SWAP_KEEPALIVE.lock().unwrap_or_else(|err| err.into_inner());
            if keepalive.is_none() {
                *keepalive = Some(replacement);
            }
            keepalive.as_ref().expect("filled just above")
        } else {
            &replacement
        };
        let stdin = std::io::stdin();
        let target_fd = match &target {
            FdSwapTarget::Stdin => stdin.as_fd(),
            FdSwapTarget::Owned(fd) => fd.as_fd(),
        };
        let saved = match nix::unistd::dup(target_fd) {
            Ok(saved) => saved,
            Err(err) => {
                log::trace!("get_events: dup for fd swap failed: {err}");
                return None;
            }
        };
        let generation = if stdin_target {
            STDIN_SWAP_GENERATION.fetch_add(1, Ordering::Relaxed)
        } else {
            0
        };
        let mut swap = Self {
            target,
            generation,
            saved: Some(saved),
        };
        let swapped = match &mut swap.target {
            FdSwapTarget::Stdin => nix::unistd::dup2_stdin(replacement),
            FdSwapTarget::Owned(fd) => nix::unistd::dup2(replacement, fd),
        };
        if let Err(err) = swapped {
            log::trace!("get_events: dup2 for fd swap failed: {err}");
            // `saved` closes when it drops; the target number still
            // points at its original description.
            swap.saved = None;
            return None;
        }
        // The replacement's own number is now redundant: the target
        // number refers to its description. For the stdin target the
        // description lives on in [`SWAP_KEEPALIVE`] (either the one just
        // adopted, or the process-lifetime one already there).
        if stdin_target {
            if let Some(saved) = swap.saved.take() {
                *SAVED_STDIN.lock().unwrap_or_else(|err| err.into_inner()) = Some(SavedStdin {
                    generation,
                    fd: saved,
                });
            }
        }
        Some(swap)
    }

    /// The swapped-in descriptor, for the caller's `poll(2)`/write use.
    fn swapped_fd<'a>(&'a self, stdin: &'a std::io::Stdin) -> BorrowedFd<'a> {
        match &self.target {
            FdSwapTarget::Stdin => stdin.as_fd(),
            FdSwapTarget::Owned(fd) => fd.as_fd(),
        }
    }
    /// Puts the original descriptor back; further calls and the later
    /// drop are no-ops.
    fn restore(&mut self) {
        match &mut self.target {
            FdSwapTarget::Stdin => {
                // Only our own generation: around an `$EDITOR` round
                // trip a newer thread may already own the stash, and
                // stealing it would silently un-swap that thread's
                // fd 0.
                restore_stdin_if_current(self.generation);
                self.saved = None;
            }
            FdSwapTarget::Owned(fd) => {
                if let Some(saved) = self.saved.take() {
                    if let Err(err) = nix::unistd::dup2(&saved, fd) {
                        log::trace!("get_events: fd swap restore failed: {err}");
                    }
                }
            }
        }
    }
}

impl Drop for FdSwap {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Opens the controlling terminal `O_RDWR | O_NONBLOCK` as an
/// independent description, for the input thread's fd 0 swap.
fn open_nonblocking_tty() -> Option<OwnedFd> {
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/tty")
    {
        Ok(file) => Some(OwnedFd::from(file)),
        Err(err) => {
            log::trace!("get_events: open /dev/tty for the stdin swap failed: {err}");
            None
        }
    }
}

/// The thread function that listens for user input and forwards it to the
/// main event loop.
///
/// Input is parsed by crossterm (`crossterm::event::poll`/`read`) and
/// translated onto meli's vocabulary by the [`crate::terminal::ratatui_bridge`]
/// module; the raw byte counterpart of each key is re-created with
/// [`encode_key`], so the `ThreadEvent::Input((Key, Vec<u8>))` contract keeps
/// receiving the same bytes the previous reader produced. Terminal
/// resizes arrive as `Event::Resize` (crossterm watches `SIGWINCH`) and are
/// forwarded through the `resize` callback; no kitty keyboard-enhancement
/// flags are pushed, only legacy sequences are expected.
///
/// # Loop structure
///
/// The main wait is meli's `poll(2)` over the tty fd and the command
/// pipe with a [`KILL_POLL_INTERVAL`] deadline; every wakeup - timeouts
/// included - consults `event::poll(ZERO)`, which is what delivers
/// The tty fd mirrors crossterm's own private `tty_fd()` resolution:
/// stdin when it is a terminal, else an `O_RDWR` `/dev/tty` of meli's
/// own - one fd serving both the `POLLIN` watch and the watchdog's DA1
/// writes (`O_WRONLY` in a `POLLIN` set is undefined behavior). In the
/// stdin case fd 0 is first swapped, via [`FdSwap`], to a fresh
/// `O_RDWR | O_NONBLOCK` `/dev/tty` description: crossterm's internal
/// drain loop only exits on `WouldBlock`, so with the original blocking
/// fd 0 a consumed-but-eventless sequence (the watchdog's whole reason
/// to exist) would park the thread inside crossterm's `read(2)` and
/// starve meli's loop of ticks and observations. The fresh open is an
/// independent description: fd 1 and fd 2 keep the original blocking
/// one, so the main thread's ratatui flush cannot `EAGAIN`, and the
/// swap is restored on every exit path as well as before every
/// interactive child spawn (see [`restore_stdin_for_child_spawn`]), so
/// children like `$EDITOR` always inherit the original blocking stdin.
/// Known limitation: when fd 0 is a tty that is *not* the controlling
/// terminal (e.g. `meli < /dev/pts/N`), the swap re-points input at the
/// controlling terminal instead of the device stdin referred to - fd 0
/// and crossterm stay self-consistent, but the original stdin device's
/// input is no longer read.
/// Because meli's poll set does not contain crossterm's `SIGWINCH`
/// waker fd, a resize is no longer delivered the moment it happens but
/// at most [`KILL_POLL_INTERVAL`] later (imperceptible in practice).
///
/// # Kill contract
///
/// If we fork (for example start `$EDITOR`) we want the `input-thread` to
/// stop reading from stdin. The best way I came up with right now is to
/// send a signal to the thread that is read in the first input in stdin
/// after the fork, and then the thread kills itself: `InputHandler::kill`
/// (`crate::state`) writes one byte to the command pipe *before* sending
/// `InputCommand::Kill` through the channel. A readable byte therefore
/// does not imply the message has landed; [`take_command`] consumes the
/// byte and then waits bounded for the message instead of probing the
/// channel non-blockingly - a message missed in that window would leave
/// nothing able to wake the poll set again and the input thread would
/// never exit. The parent process spawns a new input thread when the
/// child returns.
///
/// # Stall watchdog
///
/// [`watchdog`] guards against crossterm 0.29's private-CSI stall: an
/// unrecognized `CSI ?` sequence makes its parser buffer every following
/// byte - keys included - inside a process-global internal reader with
/// no public API to clear it, until some final byte it *does* recognize
/// flushes the buffer wholesale. The loop feeds the watchdog three kinds
/// of news: every delivered key/mouse/paste/resize event is health; a
/// cycle where meli's `poll(2)` saw the tty readable while
/// `event::poll(ZERO)` answered nothing is swallow evidence; the end of
/// each iteration is a tick decision. Once evidence is latched, no event
/// has arrived for [`watchdog::T_STALL`], tty input has been quiet for
/// [`watchdog::T_QUIET`] and the last injection is
/// [`watchdog::T_CONFIRM`] old, the loop writes a DA1 query (`ESC[c`)
/// to the tty: the terminal's reply (`CSI ? ...;c`) is a sequence
/// crossterm recognizes, and it flushes the wedged buffer. The attempt
/// is booked by the state machine itself, which is where the
/// anti-storm semantics live: after a heal no fresh evidence arrives,
/// so the watchdog never injects again; a terminal that stays dead
/// while the user keeps typing earns
/// [`watchdog::MAX_FAILED_INJECTIONS`] attempts and is then given up
/// for the rest of the process; if the user stops typing, no fresh
/// evidence means no retries.
///
/// Watchdog state - like crossterm's wedged buffer - is process-global:
/// `$EDITOR` round-trips re-spawn this thread through
/// `InputHandler::restore` (`crate::state`), and neither the buffer nor
/// a `Disabled` (or halted, see [`degraded_get_events`]) watchdog is
/// reset or revived by that. Keys swallowed during the stall are
/// discarded with the buffer flush and cannot be recovered; what the
/// watchdog restores is responsiveness for keys typed *after* it, in
/// the worst case [`watchdog::T_STALL`] + [`watchdog::T_QUIET`] plus
/// the DA1 round trip later. A healthy input path never latches
/// evidence and never sees an injection.
///
/// # Degraded mode
///
/// The watchdog is halted for the process and the loop degrades to
/// [`degraded_get_events`] - a [`KILL_POLL_INTERVAL`] `poll(2)` tick over
/// the command pipe that keeps servicing kills - in three situations:
/// stdin is not a terminal and meli's `/dev/tty` cannot be opened; the
/// fd 0 nonblocking swap cannot be set up; or the tty hangs up
/// (`POLLHUP`/`POLLERR`, which would re-wake `poll(2)` immediately and
/// spin). crossterm is consulted for events/resizes only as long as it
/// answers, never after a hang-up, and never in the swap-failure case
/// (fd 0 would still be blocking, exposing the very wedge the swap
/// exists to prevent). What the watchdog needs - observing tty
/// readiness - is exactly what this mode cannot do; when stdin is not a
/// terminal at all, crossterm reads a `/dev/tty` of its own that meli
/// cannot reach, so the watchdog's observation stays limited there even
/// outside degraded mode (known limitation).
///
/// The main loop uses [`crate::state::State::try_wait_on_children`] to check if
/// child has exited.
pub fn get_events(
    mut closure: impl FnMut((Key, Vec<u8>)),
    mut resize: impl FnMut(u16, u16),
    rx: &Receiver<InputCommand>,
    new_command_fd: &OwnedFd,
    working: std::sync::Arc<()>,
) {
    let stdin = std::io::stdin();
    let stdin_is_tty = match nix::unistd::isatty(stdin.as_fd()) {
        Ok(is_tty) => is_tty,
        Err(err) => {
            log::trace!("get_events: isatty(stdin) error: {err}");
            false
        }
    };
    // `(fd 0 swap guard, private /dev/tty fd)`: exactly one is `Some`
    // when the loop runs - the swap guard for the stdin-is-a-tty case
    // (held for the whole loop, restoring fd 0 on every exit path), the
    // private fd otherwise.
    let (stdin_swap, dev_tty) = if stdin_is_tty {
        // crossterm reads fd 0 itself, and its internal drain loop only
        // exits on `WouldBlock` - see [`FdSwap`]. Swap fd 0 to a fresh
        // `O_RDWR | O_NONBLOCK` `/dev/tty` description (fd 1/2 keep the
        // original one) so a consumed-but-eventless byte sequence cannot
        // park the thread inside crossterm's blocking `read(2)`.
        match open_nonblocking_tty()
            .and_then(|replacement| FdSwap::install(FdSwapTarget::Stdin, replacement))
        {
            Some(swap) => (Some(swap), None),
            None => {
                // fd 0 is still the original *blocking* tty: consulting
                // crossterm could wedge this thread the very way the
                // swap was meant to prevent, so the degraded loop below
                // must not ask crossterm for events (`Never`).
                halt_watchdog();
                log::warn!(
                    "input watchdog: stdin nonblocking tty swap failed; \
                     degrading to tick polling"
                );
                return degraded_get_events(
                    closure,
                    resize,
                    rx,
                    new_command_fd,
                    CrosstermConsultation::Never,
                );
            }
        }
    } else {
        // `O_RDWR`, not just `O_WRONLY`: one fd serves both the `POLLIN`
        // watch and the watchdog's DA1 writes, and an `O_WRONLY` fd in a
        // `POLLIN` set is undefined behavior. meli cannot make crossterm
        // read through this fd: it opens its own `/dev/tty`, whose
        // blocking state - and thus its exposure to the drain-loop wedge
        // - is out of meli's reach (known limitation).
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
        {
            Ok(file) => (None, Some(file)),
            Err(err) => {
                // Without a tty fd there is nothing to poll for
                // readiness or inject the DA1 query into;
                // `degraded_get_events` keeps kill/resize/event
                // delivery alive.
                halt_watchdog();
                log::warn!("input watchdog: {err}; degrading to tick polling");
                return degraded_get_events(
                    closure,
                    resize,
                    rx,
                    new_command_fd,
                    CrosstermConsultation::UntilFirstError,
                );
            }
        }
    };
    let tty_fd: BorrowedFd<'_> = match &stdin_swap {
        // fd 0, now the swapped-in nonblocking tty description.
        Some(swap) => swap.swapped_fd(&stdin),
        None => match &dev_tty {
            Some(file) => file.as_fd(),
            None => stdin.as_fd(),
        },
    };
    let poll_timeout = PollTimeout::try_from(KILL_POLL_INTERVAL)
        .expect("KILL_POLL_INTERVAL fits in a poll(2) timeout");
    'poll_while: loop {
        // Main wait: `poll(2)` over the tty and the command pipe. `EINTR`
        // (crossterm's non-`SA_RESTART` signal handlers, debuggers, ...)
        // counts as a timeout - revents are undefined there, so the fd
        // checks below are skipped and only the crossterm consultation
        // runs, which is also how a signal-driven resize stays prompt.
        let mut poll_fds = [
            PollFd::new(tty_fd, PollFlags::POLLIN),
            PollFd::new(new_command_fd.as_fd(), PollFlags::POLLIN),
        ];
        let interrupted = match poll(&mut poll_fds, poll_timeout) {
            Ok(_n_ready) => false,
            Err(Errno::EINTR) => true,
            Err(err) => {
                log::trace!("get_events: poll(2) error: {err}");
                break 'poll_while;
            }
        };
        if !interrupted {
            // The command pipe first: a kill wins over everything else.
            // `POLLHUP`/`POLLERR` count as serviceable: at shutdown the
            // write end is dropped and the read end hangs up (Linux
            // reports `POLLHUP`, not `POLLIN`), which `take_command`
            // resolves through the channel disconnect.
            if poll_fds[1].revents().is_some_and(|ev| {
                ev.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR)
            }) && take_command(rx, new_command_fd)
            {
                return;
            }
            if poll_fds[0]
                .revents()
                .is_some_and(|ev| ev.intersects(PollFlags::POLLHUP | PollFlags::POLLERR))
            {
                // The terminal went away; POLLHUP stays set and would
                // re-wake `poll(2)` immediately, so the tty leaves the
                // poll set for the rest of the process.
                halt_watchdog();
                log::warn!("input watchdog: tty POLLHUP/POLLERR; degrading to tick polling");
                return degraded_get_events(
                    closure,
                    resize,
                    rx,
                    new_command_fd,
                    CrosstermConsultation::Never,
                );
            }
        }
        // Consult crossterm on every wakeup - timeouts included - to
        // drain what it holds (parsed keys, `SIGWINCH` resizes).
        match event::poll(Duration::ZERO) {
            Ok(true) => {
                if drain_events(&mut closure, &mut resize) {
                    // Re-check the command pipe (input thread kill)
                    // between events, like the previous loop did.
                    continue 'poll_while;
                }
            }
            Ok(false) => {
                if !interrupted
                    && poll_fds[0]
                        .revents()
                        .is_some_and(|ev| ev.contains(PollFlags::POLLIN))
                {
                    // meli saw tty bytes; crossterm consumed them and
                    // answered with no event: swallow evidence.
                    log::debug!(
                        "input watchdog: tty bytes seen but crossterm delivered no event \
                         (swallow evidence)"
                    );
                    let _ = observe_watchdog(watchdog::Observation::InputConsumedNoEvent {
                        now: Instant::now(),
                    });
                }
            }
            Err(err) => {
                log::trace!("get_events: crossterm poll error: {err}");
            }
        }
        // Periodic decision point, timeouts included.
        if matches!(
            observe_watchdog(watchdog::Observation::Tick {
                now: Instant::now()
            }),
            watchdog::Decision::ShouldInject
        ) {
            log::warn!("input watchdog: injecting DA1 query (ESC[c) after stalled input parser");
            // fd 0 is the swapped-in `O_NONBLOCK` description, so the
            // 3-byte query can in theory report `EAGAIN` even though a
            // tty should always take it; retry once before giving up so
            // a transient `EAGAIN` does not burn one of the three
            // failure strikes.
            match nix::unistd::write(tty_fd, b"\x1b[c") {
                Err(Errno::EAGAIN) => {
                    if let Err(err) = nix::unistd::write(tty_fd, b"\x1b[c") {
                        log::warn!("input watchdog: DA1 query write failed: {err}");
                    }
                }
                Err(err) => {
                    log::warn!("input watchdog: DA1 query write failed: {err}");
                }
                Ok(_) => {}
            }
            if watchdog_disabled() {
                log::error!(
                    "input watchdog: {} DA1 injections without recovery; disabling for this process",
                    watchdog::MAX_FAILED_INJECTIONS
                );
            }
        }
    }
    drop(working);
}

/// Watchdog for the crossterm private-CSI input stall: latch swallow
/// evidence, judge over time windows, and heal by injecting a DA1 query.
///
/// crossterm 0.29 parses an unrecognized *private* CSI sequence (`CSI ? ...`
/// that does not end in `u` or `c`) as "incomplete" and keeps buffering every
/// byte that follows - keys included - inside the process-global internal
/// event reader, producing no event and offering no public API to clear it.
/// A late mode-report reply from the terminal or a mux is enough to wedge
/// input this way. The buffer is dropped wholesale the moment a final byte
/// crossterm *does* recognize arrives, so meli can un-wedge itself by asking
/// the terminal for one: the reply to a DA1 query (`ESC[c`) is `CSI ? ...;c`,
/// which crossterm recognizes and which flushes the buffer.
///
/// [`watchdog::Watchdog::observe`] is a pure state machine - time arrives as
/// an [`Instant`] on every observation and is never read from a clock - fed
/// by the input loop with three kinds of news:
///
/// - [`watchdog::Observation::EventDelivered`]: any key/mouse/paste/resize
///   event made it through. Clears the latched swallow evidence, refreshes
///   the event baseline and zeroes the failure count: the parser is healthy.
/// - [`watchdog::Observation::InputConsumedNoEvent`]: meli's `poll(2)` saw
///   the tty readable while crossterm's `event::poll(ZERO)` returned no
///   event, i.e. bytes were consumed and swallowed. Refreshes the activity
///   baseline and latches swallow evidence - unless it lands within
///   [`watchdog::T_CONFIRM`] of the last injection: that swallow is the DA1
///   reply draining, and latching it would re-arm a healed stall.
/// - [`watchdog::Observation::Tick`]: the periodic decision point. Answers
///   [`watchdog::Decision::ShouldInject`] only when *all* of these hold:
///
///   1. no event for [`watchdog::T_STALL`] (a stall, not a bursty stream);
///   2. swallow evidence latched since the last event;
///   3. no input activity for [`watchdog::T_QUIET`] (a dead parse, not a
///      slow paste still streaming in);
///   4. [`watchdog::T_CONFIRM`] since the last injection, or no injection
///      yet;
///   5. still [`watchdog::State::Armed`].
///
/// Answering [`watchdog::Decision::ShouldInject`] also *books* the attempt:
/// evidence consumed, failure count bumped, injection time recorded; after
/// [`watchdog::MAX_FAILED_INJECTIONS`] attempts with no delivered event in
/// between, the watchdog goes [`watchdog::State::Disabled`] for the rest of
/// the process. The resulting behavior:
///
/// - *healed, then idle*: the injection flushes the buffer, the DA1 reply's
///   own swallow falls inside the confirmation window and is not latched,
///   and with no new evidence the watchdog never injects again;
/// - *dead terminal, user keeps typing*: every burst latches fresh evidence,
///   so the watchdog retries - three failures and it stays disabled;
/// - *dead terminal, user stops*: no new evidence, no retries.
///
/// Keys swallowed during the stall are discarded together with the buffer
/// flush and cannot be recovered; what the watchdog restores is
/// responsiveness for the keys typed after it, in the worst case
/// [`watchdog::T_STALL`] + [`watchdog::T_QUIET`] plus the DA1 round trip
/// later. A healthy input path never satisfies the conditions above and
/// never sees an injection.
mod watchdog {
    use std::{
        sync::{LazyLock, Mutex},
        time::{Duration, Instant},
    };

    /// How long without any delivered event (key/mouse/paste/resize) before
    /// the input side counts as *stalled* rather than merely idle.
    pub(super) const T_STALL: Duration = Duration::from_secs(3);

    /// How long without any tty input activity before latched swallow
    /// evidence is trusted: distinguishes a dead parse from a slow paste
    /// that is still streaming in.
    pub(super) const T_QUIET: Duration = Duration::from_secs(2);

    /// Round-trip window for a DA1 query: swallows observed this soon after
    /// an injection are the reply itself and do not count as new evidence,
    /// which is what keeps a healed stall from re-arming (and the watchdog
    /// from storming the terminal).
    pub(super) const T_CONFIRM: Duration = Duration::from_secs(1);

    /// DA1 injections without a delivered event in between after which the
    /// watchdog disables itself for the rest of the process.
    pub(super) const MAX_FAILED_INJECTIONS: u32 = 3;

    /// Lifecycle of the watchdog.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum State {
        /// Injecting is allowed.
        Armed,
        /// Terminal after [`MAX_FAILED_INJECTIONS`] failed injections;
        /// nothing revives it.
        Disabled,
    }

    /// What the input loop reports to [`Watchdog::observe`].
    #[derive(Clone, Copy, Debug)]
    pub(super) enum Observation {
        /// Any input event (key/mouse/paste/resize) was delivered: the
        /// parser is healthy.
        EventDelivered { now: Instant },
        /// The tty was readable per meli's `poll(2)` while crossterm's
        /// `event::poll(ZERO)` produced no event: bytes were consumed and
        /// swallowed.
        InputConsumedNoEvent { now: Instant },
        /// Periodic decision point.
        Tick { now: Instant },
    }

    /// What the watchdog asks of the input loop for one observation.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    #[must_use]
    pub(super) enum Decision {
        /// Write the DA1 query `ESC[c` to the tty; the attempt is already
        /// booked (see [`Watchdog::observe`]).
        ShouldInject,
        /// Do nothing.
        NoAction,
    }

    /// The watchdog state machine.
    ///
    /// All decisions are functions of this state plus the `now` each
    /// observation carries; nothing here reads a clock.
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Watchdog {
        state: State,
        last_event: Instant,
        last_input_activity: Instant,
        last_injection: Option<Instant>,
        swallow_pending: bool,
        failed_injections: u32,
    }

    impl Watchdog {
        /// A freshly armed watchdog whose baselines are `now`.
        ///
        /// `last_event` and `last_input_activity` start at construction
        /// time, so a quiet process is *idle*, never *stalled*: evidence and
        /// the [`T_STALL`] window are both needed before a first injection
        /// can ever fire.
        pub(super) fn armed(now: Instant) -> Self {
            Self {
                state: State::Armed,
                last_event: now,
                last_input_activity: now,
                last_injection: None,
                swallow_pending: false,
                failed_injections: 0,
            }
        }

        /// The current [`State`]; `Disabled` is terminal for the process.
        pub(super) fn state(&self) -> State {
            self.state
        }

        /// Folds one observation into the machine and answers it.
        ///
        /// A [`Decision::ShouldInject`] answer means the attempt has already
        /// been booked - evidence consumed, failure count bumped,
        /// `last_injection` recorded - and the caller only owes the terminal
        /// the 3-byte query `ESC[c`.
        pub(super) fn observe(&mut self, observation: Observation) -> Decision {
            match observation {
                Observation::EventDelivered { now } => {
                    self.swallow_pending = false;
                    self.last_event = now;
                    self.failed_injections = 0;
                    Decision::NoAction
                }
                Observation::InputConsumedNoEvent { now } => {
                    self.last_input_activity = now;
                    // Swallows inside the confirmation window are the DA1
                    // reply itself draining; latching them would re-arm a
                    // healed stall and storm the terminal.
                    if self.elapsed_since_last_injection(now) >= T_CONFIRM {
                        self.swallow_pending = true;
                    }
                    Decision::NoAction
                }
                Observation::Tick { now } => {
                    if self.state == State::Armed
                        && self.swallow_pending
                        && now.duration_since(self.last_event) >= T_STALL
                        && now.duration_since(self.last_input_activity) >= T_QUIET
                        && self.elapsed_since_last_injection(now) >= T_CONFIRM
                    {
                        self.attempt_injection(now);
                        Decision::ShouldInject
                    } else {
                        Decision::NoAction
                    }
                }
            }
        }

        /// Books one DA1 injection attempt: consumes the latched evidence,
        /// bumps the consecutive-failure count and records when the query
        /// left. After [`MAX_FAILED_INJECTIONS`] attempts without a
        /// delivered event in between, the watchdog disables itself.
        fn attempt_injection(&mut self, now: Instant) {
            self.swallow_pending = false;
            self.failed_injections += 1;
            self.last_injection = Some(now);
            if self.failed_injections >= MAX_FAILED_INJECTIONS {
                self.state = State::Disabled;
            }
        }

        /// Time since the last injection; with none yet, the window is
        /// always satisfied.
        fn elapsed_since_last_injection(&self, now: Instant) -> Duration {
            self.last_injection
                .map_or(Duration::MAX, |then| now.duration_since(then))
        }
    }

    /// The one watchdog instance of the process.
    ///
    /// `$EDITOR` round-trips make `InputHandler::restore` (`crate::state`)
    /// re-spawn the input thread, and crossterm's stalled internal buffer is
    /// a process global such a restart does not clear either - so the
    /// watchdog state must share that lifetime: it lives here, at process
    /// scope, and a thread restart must neither reset its baselines nor
    /// revive a `Disabled` instance. [`LazyLock`] hands every input thread
    /// the same machine.
    static WATCHDOG: LazyLock<Mutex<Watchdog>> =
        LazyLock::new(|| Mutex::new(Watchdog::armed(Instant::now())));

    /// The process-global [`Watchdog`], created lazily on first access.
    pub(super) fn global_watchdog() -> &'static Mutex<Watchdog> {
        &WATCHDOG
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::{Duration, Instant};

        /// Seconds, for readable scenario offsets.
        const fn secs(n: u64) -> Duration {
            Duration::from_secs(n)
        }

        /// Milliseconds, for sub-second scenario offsets.
        const fn msecs(n: u64) -> Duration {
            Duration::from_millis(n)
        }

        /// An [`Observation`] kind; the runner timestamps it.
        #[derive(Clone, Copy, Debug)]
        enum Kind {
            /// [`Observation::EventDelivered`].
            Event,
            /// [`Observation::InputConsumedNoEvent`].
            Swallow,
            /// [`Observation::Tick`].
            Tick,
        }

        impl Kind {
            fn at(self, now: Instant) -> Observation {
                match self {
                    Self::Event => Observation::EventDelivered { now },
                    Self::Swallow => Observation::InputConsumedNoEvent { now },
                    Self::Tick => Observation::Tick { now },
                }
            }
        }

        /// One scenario step: feed `kind` at `base + at`, expecting
        /// `injects`.
        struct Step {
            at: Duration,
            kind: Kind,
            injects: bool,
        }

        /// A scenario: ordered steps plus the state expected at the end.
        struct Case {
            name: &'static str,
            steps: Vec<Step>,
            end: State,
        }

        fn step(at: Duration, kind: Kind, injects: bool) -> Step {
            Step { at, kind, injects }
        }

        /// Runs `case` against a fresh watchdog anchored at a real
        /// [`Instant`]; time is driven only by the step offsets, nothing
        /// sleeps.
        fn run(case: &Case) {
            let base = Instant::now();
            let mut watchdog = Watchdog::armed(base);
            for (i, step) in case.steps.iter().enumerate() {
                assert_eq!(
                    watchdog.observe(step.kind.at(base + step.at)),
                    if step.injects {
                        Decision::ShouldInject
                    } else {
                        Decision::NoAction
                    },
                    "{}: step #{} ({:?} at {:?})",
                    case.name,
                    i + 1,
                    step.kind,
                    step.at,
                );
            }
            assert_eq!(watchdog.state(), case.end, "{}: end state", case.name);
        }

        /// U1: an event every second with ticks in between. There is never
        /// any swallow evidence, so a healthy stream cannot inject.
        fn healthy_stream_steps() -> Vec<Step> {
            let mut steps = Vec::new();
            for i in 0..30u64 {
                steps.push(step(secs(i), Kind::Event, false));
                steps.push(step(secs(i) + msecs(500), Kind::Tick, false));
            }
            steps
        }

        #[test]
        fn scenario_table() {
            let cases = vec![
                Case {
                    name: "U1: healthy input stream never injects",
                    steps: healthy_stream_steps(),
                    end: State::Armed,
                },
                Case {
                    name: "U2: idle stall with one swallow injects exactly once",
                    steps: vec![
                        step(Duration::ZERO, Kind::Swallow, false),
                        // Exactly T_STALL + T_QUIET after the baselines.
                        step(T_STALL + T_QUIET, Kind::Tick, true),
                        // The attempt consumed the latch: no new evidence,
                        // no second injection, however far time advances.
                        step(secs(6), Kind::Tick, false),
                        step(secs(600), Kind::Tick, false),
                    ],
                    end: State::Armed,
                },
                Case {
                    name: "U3: swallow inside the confirmation window is not evidence",
                    steps: vec![
                        step(Duration::ZERO, Kind::Swallow, false),
                        step(T_STALL + T_QUIET, Kind::Tick, true),
                        // 0.5s after the injection: the DA1 reply draining.
                        step(T_STALL + T_QUIET + T_CONFIRM / 2, Kind::Swallow, false),
                        // Every other condition holds by now (quiet for
                        // 4.5s, last injection 5s ago); only the unlatched
                        // evidence keeps this from injecting.
                        step(secs(10), Kind::Tick, false),
                    ],
                    end: State::Armed,
                },
                Case {
                    name: "U4: event after injection resets the failure count",
                    steps: vec![
                        step(Duration::ZERO, Kind::Swallow, false),
                        step(secs(5), Kind::Tick, true),
                        // Exactly T_CONFIRM after the 5s injection: the
                        // swallow must still count as evidence.
                        step(secs(6), Kind::Swallow, false),
                        step(secs(9) + msecs(500), Kind::Tick, true),
                        // Delivered event: latch cleared, baselines
                        // refreshed, failure count zeroed.
                        step(secs(10), Kind::Event, false),
                        step(secs(13) + msecs(500), Kind::Swallow, false),
                        step(secs(16) + msecs(500), Kind::Tick, true),
                        step(secs(17) + msecs(600), Kind::Swallow, false),
                        // Had the count survived the event, the previous
                        // injection would have been the disabling third and
                        // this could not fire; it is only the second.
                        step(secs(21), Kind::Tick, true),
                    ],
                    end: State::Armed,
                },
                Case {
                    name: "U4a: event clears latched swallow evidence",
                    steps: vec![
                        step(Duration::ZERO, Kind::Swallow, false),
                        step(msecs(500), Kind::Event, false),
                        // Stall (3.5s) and quiet (4s) would both be met
                        // here; only the cleared latch keeps this from
                        // injecting.
                        step(secs(4), Kind::Tick, false),
                    ],
                    end: State::Armed,
                },
                Case {
                    name: "U4b: event refreshes the stall baseline",
                    steps: vec![
                        step(Duration::ZERO, Kind::Swallow, false),
                        step(msecs(500), Kind::Event, false),
                        step(secs(1), Kind::Swallow, false),
                        // Only 2.9s since the event, under T_STALL;
                        // measured against the construction baseline it
                        // would be 3.4s and mis-inject.
                        step(secs(3) + msecs(400), Kind::Tick, false),
                    ],
                    end: State::Armed,
                },
                Case {
                    name: "U5: three failed injections disable the watchdog for good",
                    steps: vec![
                        step(Duration::ZERO, Kind::Swallow, false),
                        step(secs(5), Kind::Tick, true),
                        step(secs(6) + msecs(500), Kind::Swallow, false),
                        step(secs(10), Kind::Tick, true),
                        step(secs(11) + msecs(500), Kind::Swallow, false),
                        // Third failure -> Disabled.
                        step(secs(15), Kind::Tick, true),
                        // Fresh evidence and arbitrarily advanced time must
                        // not revive it.
                        step(secs(16) + msecs(500), Kind::Swallow, false),
                        step(secs(600), Kind::Tick, false),
                    ],
                    end: State::Disabled,
                },
                Case {
                    name: "U6: slow paste (swallows under T_QUIET apart) never injects",
                    steps: vec![
                        step(Duration::ZERO, Kind::Swallow, false),
                        step(secs(1) + msecs(500), Kind::Swallow, false),
                        step(secs(3), Kind::Swallow, false),
                        step(secs(3) + msecs(250), Kind::Tick, false),
                        step(secs(4) + msecs(500), Kind::Swallow, false),
                        step(secs(6), Kind::Swallow, false),
                        step(secs(6) + msecs(250), Kind::Tick, false),
                        step(secs(7) + msecs(500), Kind::Swallow, false),
                        step(secs(9), Kind::Swallow, false),
                        step(secs(9) + msecs(250), Kind::Tick, false),
                    ],
                    end: State::Armed,
                },
                Case {
                    name: "U7: swallowed typing burst, then T_QUIET of quiet, injects",
                    steps: {
                        let mut steps = Vec::new();
                        // Frantic typing, every key swallowed: activity
                        // stays fresh, so ticks interleaved with the burst
                        // never fire.
                        for i in 0..=20u64 {
                            let at = msecs(i * 300);
                            steps.push(step(at, Kind::Swallow, false));
                            if i % 7 == 3 {
                                steps.push(step(at + msecs(100), Kind::Tick, false));
                            }
                        }
                        // Hands off at t = 6s; exactly T_QUIET later the
                        // stall is confirmed.
                        steps.push(step(secs(8), Kind::Tick, true));
                        steps
                    },
                    end: State::Armed,
                },
                Case {
                    name: "U11: healed stall stays quiet forever, still Armed",
                    steps: vec![
                        step(Duration::ZERO, Kind::Swallow, false),
                        step(secs(5), Kind::Tick, true),
                        // The DA1 reply swallowed inside the window: not new
                        // evidence.
                        step(secs(5) + msecs(500), Kind::Swallow, false),
                        step(secs(10), Kind::Tick, false),
                        step(secs(20), Kind::Tick, false),
                        step(secs(100), Kind::Tick, false),
                        step(secs(3_600), Kind::Tick, false),
                    ],
                    end: State::Armed,
                },
            ];
            for case in &cases {
                run(case);
            }
        }

        /// U12: the watchdog must be process-global. `InputHandler::restore`
        /// re-spawns the input thread around `$EDITOR` round-trips, and the
        /// crossterm stall buffer it mirrors survives those restarts - so a
        /// `Disabled` global must survive them too, not come back Armed.
        #[test]
        fn u12_disabled_global_survives_input_thread_restart() {
            let base = Instant::now();
            {
                // First "input thread": three failed injections drive the
                // process-global instance to Disabled, each attempt with
                // fresh evidence in between.
                let mut watchdog = global_watchdog().lock().unwrap();
                let ev = |d| Observation::InputConsumedNoEvent { now: base + d };
                let tick = |d| Observation::Tick { now: base + d };
                assert_eq!(watchdog.observe(ev(Duration::ZERO)), Decision::NoAction);
                assert_eq!(watchdog.observe(tick(secs(5))), Decision::ShouldInject);
                assert_eq!(
                    watchdog.observe(ev(secs(6) + msecs(500))),
                    Decision::NoAction
                );
                assert_eq!(watchdog.observe(tick(secs(10))), Decision::ShouldInject);
                assert_eq!(
                    watchdog.observe(ev(secs(11) + msecs(500))),
                    Decision::NoAction
                );
                assert_eq!(watchdog.observe(tick(secs(15))), Decision::ShouldInject);
                assert_eq!(watchdog.state(), State::Disabled);
            }
            // Guard dropped: the thread exits the way it does around
            // `$EDITOR` round-trips.

            // The re-spawned thread fetches the very same instance: still
            // Disabled, and fresh evidence plus arbitrarily advanced time do
            // not revive it.
            let mut watchdog = global_watchdog().lock().unwrap();
            assert_eq!(watchdog.state(), State::Disabled);
            let ev = |d| Observation::InputConsumedNoEvent { now: base + d };
            let tick = |d| Observation::Tick { now: base + d };
            assert_eq!(watchdog.observe(ev(secs(20))), Decision::NoAction);
            assert_eq!(watchdog.observe(tick(secs(3_600))), Decision::NoAction);
            // Restore a fresh machine for whatever runs after this test:
            // the global is process-wide, and leaving it Disabled would
            // make later wiring tests order-dependent.
            *watchdog = Watchdog::armed(base);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Tests that touch [`SAVED_STDIN`] must not overlap (test threads
    /// run in parallel): one test's seeded stash must never be taken -
    /// and thereby dup2'd onto fd 0 - by another.
    static STDIN_STASH_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// `take_command` recognizes a kill: byte first, then the message -
    /// the order `InputHandler::kill` writes them in.
    #[test]
    fn take_command_delivers_kill() {
        let (tx, rx) = crossbeam::channel::unbounded::<InputCommand>();
        let (read_fd, write_fd) = nix::unistd::pipe().unwrap();
        nix::unistd::write(&write_fd, &[1]).unwrap();
        tx.send(InputCommand::Kill).unwrap();
        assert!(take_command(&rx, &read_fd));
    }

    /// The byte-without-message window must wait for the message instead
    /// of probing the channel non-blockingly (which would drop the
    /// command and strand the input thread).
    #[test]
    fn take_command_waits_for_late_message() {
        let (tx, rx) = crossbeam::channel::unbounded::<InputCommand>();
        let (read_fd, write_fd) = nix::unistd::pipe().unwrap();
        nix::unistd::write(&write_fd, &[1]).unwrap();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            tx.send(InputCommand::Kill).unwrap();
        });
        let started = Instant::now();
        assert!(take_command(&rx, &read_fd));
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "must block on recv_timeout until the message lands, not return early"
        );
        sender.join().unwrap();
        drop(write_fd);
    }

    /// Shutdown shape: the sender is dropped and the pipe write end with
    /// it, leaving the read end at EOF (`POLLIN` permanently ready). A
    /// disconnected channel must count as a kill - the previous loop
    /// exited through `unwrap_or_default` here - or the caller would
    /// spin on the forever-readable pipe.
    #[test]
    fn take_command_treats_disconnected_channel_as_kill() {
        let (tx, rx) = crossbeam::channel::unbounded::<InputCommand>();
        let (read_fd, write_fd) = nix::unistd::pipe().unwrap();
        drop(tx);
        drop(write_fd);
        let started = Instant::now();
        assert!(take_command(&rx, &read_fd));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "disconnection must resolve immediately, without the full \
             COMMAND_RECV_TIMEOUT wait"
        );
    }

    /// The degraded loop must exit through its kill service on the same
    /// EOF'd-pipe-plus-disconnected-channel shape instead of spinning on
    /// it: its rhythm is the `poll(2)` over the command pipe, so the
    /// wake is serviced within one [`KILL_POLL_INTERVAL`] tick.
    #[test]
    fn degraded_loop_exits_instead_of_spinning_on_eof_pipe() {
        let (tx, rx) = crossbeam::channel::unbounded::<InputCommand>();
        let (read_fd, write_fd) = nix::unistd::pipe().unwrap();
        drop(tx);
        drop(write_fd);
        let started = Instant::now();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            degraded_get_events(
                |_| {},
                |_, _| {},
                &rx,
                &read_fd,
                CrosstermConsultation::UntilFirstError,
            );
            done_tx.send(()).expect("test channel alive");
        });
        done_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("degraded loop must exit via the kill service");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    /// Reads one byte from `fd` (blocking dup of a pipe read end).
    fn read_one_byte(fd: &OwnedFd) -> u8 {
        let mut byte = [0; 1];
        nix::unistd::read(fd, &mut byte).expect("one byte is buffered");
        byte[0]
    }

    /// A spare, high fd number swap: the target's descriptor must point
    /// at the replacement's open file description while swapped and back
    /// at the original after [`FdSwap::restore`]/drop. fd 0 is never
    /// touched here.
    #[test]
    fn fd_swap_roundtrip_on_spare_fd() {
        let (orig_read, orig_write) = nix::unistd::pipe().unwrap();
        let (other_read, other_write) = nix::unistd::pipe().unwrap();
        nix::unistd::write(&orig_write, b"O").unwrap();
        nix::unistd::write(&other_write, b"R").unwrap();

        let stdin = std::io::stdin();
        let mut swap =
            FdSwap::install(FdSwapTarget::Owned(orig_read), other_read).expect("swap installs");
        let swapped = nix::unistd::dup(swap.swapped_fd(&stdin)).unwrap();
        assert_eq!(
            read_one_byte(&swapped),
            b'R',
            "target now reads the replacement"
        );
        drop(swapped);

        swap.restore();
        let restored = nix::unistd::dup(swap.swapped_fd(&stdin)).unwrap();
        assert_eq!(
            read_one_byte(&restored),
            b'O',
            "target reads its original again"
        );
        drop(restored);
    }

    /// Restoring twice and dropping afterwards must all be safe: the
    /// second restore is a no-op (`saved` is taken), which is what makes
    /// the guard's drop and a child-spawn restore race-free.
    #[test]
    fn fd_swap_restore_is_idempotent() {
        let (orig_read, orig_write) = nix::unistd::pipe().unwrap();
        let (other_read, other_write) = nix::unistd::pipe().unwrap();
        nix::unistd::write(&orig_write, b"O").unwrap();
        nix::unistd::write(&other_write, b"R").unwrap();

        let stdin = std::io::stdin();
        let mut swap =
            FdSwap::install(FdSwapTarget::Owned(orig_read), other_read).expect("swap installs");
        swap.restore();
        swap.restore();
        let restored = nix::unistd::dup(swap.swapped_fd(&stdin)).unwrap();
        assert_eq!(read_one_byte(&restored), b'O');
        drop(restored);
        drop(swap);
    }

    /// `restore_stdin_for_child_spawn` runs before *every* interactive
    /// spawn, including when no swap was ever installed (e.g. stdin was
    /// never a tty): it must be a no-op then.
    #[test]
    fn restore_stdin_without_swap_is_noop() {
        let _lock = STDIN_STASH_TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        assert!(SAVED_STDIN
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_none());
        restore_stdin_for_child_spawn();
    }

    /// An old generation's guard drop must not steal the stash a newer
    /// swap owns: around an `$EDITOR` round trip the re-spawned input
    /// thread T2 installs its swap before T1 exits, and T1's drop
    /// stealing T2's entry would silently re-point fd 0 at the blocking
    /// description while T2 believes it swapped. Only the
    /// generation-matching (or unconditional, main-thread) restore may
    /// touch the stash - which is also why this test seeds a fake stash
    /// and asserts the old generation's restore is a no-op instead of
    /// exercising the matching path (that would dup2 onto fd 0).
    #[test]
    fn old_generation_restore_does_not_steal_newer_stash() {
        let _lock = STDIN_STASH_TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let (read_end, write_end) = nix::unistd::pipe().unwrap();
        *SAVED_STDIN.lock().unwrap_or_else(|err| err.into_inner()) = Some(SavedStdin {
            generation: 7,
            fd: read_end,
        });
        // The old guard's generation-checked restore: no-op, the stash
        // survives untouched.
        restore_stdin_if_current(3);
        let stashed = SAVED_STDIN
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        let stashed = stashed.expect("old generation must not steal the newer stash");
        assert_eq!(stashed.generation, 7);
        // The stashed fd was not closed behind the stash's back: its
        // pipe peer still accepts a write.
        nix::unistd::write(&write_end, &[1]).expect("stashed fd must still be open");
    }
}
