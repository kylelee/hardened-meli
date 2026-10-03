//
// meli
//
// Copyright 2017-2018 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

//! Signal handling on the `tokio` runtime.
//!
//! Each watched signal becomes an infrastructure task on the job
//! executor's runtime, parked on [`tokio::signal::unix::Signal`] and woken
//! by the runtime's signal driver exactly when the signal is delivered:
//! no polling thread, no 100ms sleep loop. The main-loop keepalive
//! heartbeat (the side job of the former poll thread) is a
//! [`tokio::time::interval`] task on the same runtime.
//!
//! Semantics preserved from the old poll-thread implementation:
//!
//! - forward each delivered signal once on a bounded (100) channel,
//!   dropping on backpressure instead of blocking - the main loop reaps
//!   children itself in `State::try_wait_on_children`, so a dropped
//!   SIGCHLD only loses a nudge, never work;
//! - beat [`ThreadEvent::Pulse`] immediately and then every 300ms, so
//!   `State::pulse` keeps running even when nothing else wakes the main
//!   loop.

use std::{os::raw::c_int, time::Duration};

use tokio::signal::unix::{signal, SignalKind};

use crate::{jobs::JobExecutor, *};

/// Watch `signals` on `executor`'s `tokio` runtime, forwarding each
/// delivery on the returned receiver, and start the 300ms main-loop
/// keepalive heartbeat on `sender`.
///
/// The tasks are spawned via [`JobExecutor::spawn_infrastructure`]: they
/// are permanent machinery, not jobs, so they never appear in the jobs
/// manager UI. They end on their own when the main loop channels close,
/// or are dropped along with the runtime at process shutdown.
///
/// The former `SIGALRM` `sigaction` + alarm pipe registration was removed
/// together with the poll thread: nothing in the workspace ever raises
/// `SIGALRM`, and the pipe's read end was dropped before the thread even
/// started, so a delivery would only ever have written into a broken
/// pipe.
pub fn notify(
    signals: &[c_int],
    sender: crossbeam::channel::Sender<ThreadEvent>,
    executor: &JobExecutor,
) -> std::result::Result<crossbeam::channel::Receiver<c_int>, std::io::Error> {
    let (s, r) = crossbeam::channel::bounded(100);
    // `tokio::signal::unix::signal` and `tokio::time::interval` must be
    // created inside the runtime's context; `enter` marks this thread as
    // part of it for the duration of the setup.
    let _runtime = executor.enter();
    for &sig in signals {
        let mut stream = signal(SignalKind::from_raw(sig))?;
        let s = s.clone();
        executor.spawn_infrastructure(async move {
            loop {
                // Resolves once per delivered signal; the stream never
                // ends.
                if stream.recv().await.is_none() {
                    return;
                }
                // Drop instead of blocking when the main loop is more
                // than 100 signals behind, like the old `send_timeout`.
                let _ = s.try_send(sig);
            }
        });
    }
    executor.spawn_infrastructure(async move {
        // The first tick completes immediately, matching the old loop
        // which beat before its first sleep.
        let mut beat = tokio::time::interval(Duration::from_millis(300));
        loop {
            beat.tick().await;
            if sender.send(ThreadEvent::Pulse).is_err() {
                // The main loop is gone; stop beating.
                return;
            }
        }
    });
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raised signal must arrive on the returned receiver event-driven,
    /// without any polling thread.
    #[test]
    fn raised_signal_is_forwarded() {
        let (sender, main_rx) = crossbeam::channel::unbounded();
        let executor = JobExecutor::new(sender.clone());
        let signal_rx = notify(&[libc::SIGCHLD], sender, &executor).unwrap();
        // SAFETY: raising SIGCHLD only runs the installed (async-signal
        // safe) handler; there is no state to corrupt.
        unsafe { libc::raise(libc::SIGCHLD) };
        // Generous timeout: the signal driver is asynchronous and tests
        // run in parallel, so delivery may take a scheduling round. A
        // spurious extra SIGCHLD from a sibling signal test is also fine.
        let mut forwarded = false;
        for _ in 0..4 {
            match signal_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(sig) if sig == libc::SIGCHLD => {
                    forwarded = true;
                    break;
                }
                Err(_timeout) => break,
                Ok(_) => {}
            }
        }
        assert!(forwarded);
        drop(executor); // runtime shutdown cancels the watcher tasks
        drop(main_rx);
    }

    /// The keepalive heartbeat beats the main loop at the old poll
    /// thread's cadence: the first beat immediately, then more.
    #[test]
    fn keepalive_pulse_beats_main_loop() {
        let (sender, main_rx) = crossbeam::channel::unbounded();
        let executor = JobExecutor::new(sender.clone());
        let _signal_rx = notify(&[libc::SIGCHLD], sender, &executor).unwrap();
        assert!(matches!(
            main_rx.recv_timeout(Duration::from_secs(2)).ok(),
            Some(ThreadEvent::Pulse)
        ));
        assert!(matches!(
            main_rx.recv_timeout(Duration::from_secs(2)).ok(),
            Some(ThreadEvent::Pulse)
        ));
    }
}
