/*
 * meli - jobs executor
 *
 * Copyright 2020 Manos Pitsidianakis
 * Copyright 2026 Kyle Lee
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

//! Async job executor backed by the `tokio` runtime.
//!
//! [`JobExecutor`] owns a multi-thread `tokio` runtime which is the async
//! backbone of the application:
//!
//! - [`IsAsync::Async`] jobs run as `tokio` tasks on the runtime's worker
//!   threads.
//! - [`IsAsync::Blocking`] jobs are wrapped in [`tokio::task::spawn_blocking`]
//!   so blocking work cannot starve async jobs.
//! - Timers ([`JobExecutor::create_timer`]) are runtime tasks that sleep with
//!   [`tokio::time::sleep`] and emit [`UIEvent::Timer`] through the main-loop
//!   channel.
//!
//! A panic inside a spawned job is isolated per task: it is caught with
//! [`futures::FutureExt::catch_unwind`], logged, the job's metadata is marked
//! as failed and the main loop is still notified with
//! [`ThreadEvent::JobFinished`]. One bad job therefore can no longer freeze
//! the UI the way a dead executor lane used to. Cancellation
//! ([`JoinHandle::cancel`]) aborts the task at its next `.await` point,
//! dropping the future and releasing its resources.

use std::{
    borrow::Cow,
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use crossbeam::channel::Sender;
pub use futures::channel::oneshot;
use futures::future::FutureExt;
use indexmap::IndexMap;
use melib::{log, utils::datetime, uuid::Uuid, UnixTimestamp};

use crate::types::{StatusEvent, ThreadEvent, UIEvent};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub enum IsAsync {
    Async,
    Blocking,
}

#[derive(Clone, Debug)]
struct FinishedTimestamp(Arc<Mutex<UnixTimestamp>>);

impl FinishedTimestamp {
    fn finished(&self) -> Option<UnixTimestamp> {
        match self.0.lock() {
            Ok(v) if *v == 0 => None,
            Ok(v) => Some(*v),
            Err(poison) => {
                let mut guard = poison.into_inner();
                if *guard == 0 {
                    *guard = UnixTimestamp::default();
                }
                Some(*guard)
            }
        }
    }

    fn set_finished(&self, new_value: Option<UnixTimestamp>) {
        let new_value = new_value.unwrap_or_default();
        match self.0.lock() {
            Ok(mut f) => *f = new_value,
            Err(poison) => {
                let mut guard = poison.into_inner();
                *guard = new_value;
            }
        }
    }
}

macro_rules! uuid_hash_type {
    ($n:ident) => {
        #[derive(PartialEq, Hash, Eq, Copy, Clone, Ord, PartialOrd, Serialize, Deserialize)]
        pub struct $n(Uuid);

        impl std::fmt::Debug for $n {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}", self.0.to_string())
            }
        }

        impl std::fmt::Display for $n {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}", self.0.to_string())
            }
        }

        impl Default for $n {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $n {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            pub fn null() -> Self {
                Self(Uuid::nil())
            }
        }
    };
}
uuid_hash_type!(JobId);
uuid_hash_type!(TimerId);

#[derive(Clone, Debug)]
/// A spawned future's metadata for book-keeping.
pub struct JobMetadata {
    id: JobId,
    desc: Cow<'static, str>,
    started: UnixTimestamp,
    finished: FinishedTimestamp,
    succeeded: bool,
}

impl JobMetadata {
    pub fn id(&self) -> &JobId {
        &self.id
    }

    pub fn description(&self) -> &str {
        &self.desc
    }

    pub fn started(&self) -> UnixTimestamp {
        self.started
    }

    pub fn finished(&self) -> Option<UnixTimestamp> {
        self.finished.finished()
    }

    pub fn succeeded(&self) -> bool {
        self.succeeded
    }
}

#[derive(Debug)]
pub struct JobExecutor {
    /// The `tokio` runtime that drives every spawned job and timer.
    runtime: tokio::runtime::Runtime,
    sender: Sender<ThreadEvent>,
    timers: Arc<Mutex<IndexMap<TimerId, TimerPrivate>>>,
    pub jobs: Arc<Mutex<IndexMap<JobId, JobMetadata>>>,
}

#[derive(Debug, Default)]
struct TimerPrivate {
    /// Interval for periodic timer.
    interval: Duration,
    /// Time until next expiration.
    value: Duration,
    active: bool,
    handle: Option<tokio::task::AbortHandle>,
}

#[derive(Debug)]
pub struct Timer {
    id: TimerId,
    job_executor: Arc<JobExecutor>,
}

impl Timer {
    pub fn id(&self) -> TimerId {
        self.id
    }

    pub fn rearm(&self) {
        self.job_executor.rearm(self.id);
    }

    pub fn disable(&self) {
        self.job_executor.disable_timer(self.id);
    }

    pub fn set_interval(&self, new_val: Duration) {
        self.job_executor.set_interval(self.id, new_val);
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.disable();
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(msg) = payload.downcast_ref::<&'static str>() {
        (*msg).to_string()
    } else if let Some(msg) = payload.downcast_ref::<String>() {
        msg.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

impl JobExecutor {
    /// Creates the executor with its dedicated multi-thread `tokio` runtime.
    ///
    /// Worker threads are named `meli-executor`. Job futures are
    /// panic-isolated per task (see [`JobExecutor::spawn`]), so no
    /// supervisor-restart logic is needed here. The runtime is dropped along
    /// with the executor, cancelling pending tasks at shutdown.
    pub fn new(sender: Sender<ThreadEvent>) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(
                std::thread::available_parallelism()
                    .map(Into::into)
                    .unwrap_or(1),
            )
            .thread_name("meli-executor")
            // The time driver is required by `tokio::time::sleep` in
            // `arm_timer`.
            .enable_all()
            .build()
            .expect("could not build the `tokio` runtime for the job executor");
        Self {
            runtime,
            sender,
            timers: Arc::new(Mutex::new(IndexMap::default())),
            jobs: Arc::new(Mutex::new(IndexMap::default())),
        }
    }

    /// Spawns a future with a generic return value `R`
    #[inline(always)]
    pub fn spawn<F, R>(
        &self,
        desc: Cow<'static, str>,
        future: F,
        is_async: IsAsync,
    ) -> JoinHandle<R>
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        if matches!(is_async, IsAsync::Async) {
            self.spawn_specialized(desc, future)
        } else {
            self.spawn_blocking(desc, future)
        }
    }

    /// Spawns a future with a generic return value `R`
    #[inline(always)]
    fn spawn_specialized<F, R>(&self, desc: Cow<'static, str>, future: F) -> JoinHandle<R>
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let finished_sender = self.sender.clone();
        let job_id = JobId::new();
        // We do not use `AtomicU64` because it's not portable, so ignore the lint.
        #[allow(clippy::mutex_integer)]
        let finished = FinishedTimestamp(Arc::new(Mutex::new(0)));
        let cancel = Arc::new(AtomicBool::new(false));

        self.jobs.lock().unwrap().insert(
            job_id,
            JobMetadata {
                id: job_id,
                desc: desc.clone(),
                started: datetime::now(),
                finished: finished.clone(),
                succeeded: true,
            },
        );

        let jobs = self.jobs.clone();
        let finished_task = finished.clone();
        let task = self.runtime.handle().spawn(async move {
            // Isolate panics per job: catch the unwind, log it, mark the job
            // as failed and still notify the main loop, so a panicking job
            // cannot freeze the UI or take the runtime down.
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(res) => {
                    let _ = sender.send(res);
                }
                Err(payload) => {
                    log::error!(
                        "job {job_id} `{desc}` panicked: {}",
                        panic_message(payload.as_ref())
                    );
                    jobs.lock().unwrap().entry(job_id).and_modify(|entry| {
                        entry.succeeded = false;
                    });
                }
            }
            if let Ok(mut guard) = finished_task.0.lock() {
                *guard = datetime::now();
            }
            let _ = finished_sender.send(ThreadEvent::JobFinished(job_id));
        });

        JoinHandle {
            abort: task.abort_handle(),
            cancel,
            finished,
            chan: receiver,
            job_id,
        }
    }

    /// Spawns a future with a generic return value `R` that might block on a
    /// new thread
    #[inline(always)]
    fn spawn_blocking<F, R>(&self, desc: Cow<'static, str>, future: F) -> JoinHandle<R>
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        self.spawn_specialized(desc, async move {
            tokio::task::spawn_blocking(move || futures::executor::block_on(future))
                .await
                .expect("blocking job worker panicked")
        })
    }

    pub fn create_timer(self: Arc<Self>, interval: Duration, value: Duration) -> Timer {
        let timer = TimerPrivate {
            interval,
            value,
            active: true,
            handle: None,
        };
        let id = TimerId::default();
        self.timers.lock().unwrap().insert(id, timer);
        self.arm_timer(id, value);
        Timer {
            id,
            job_executor: self,
        }
    }

    pub fn rearm(&self, timer_id: TimerId) {
        let mut timers_lck = self.timers.lock().unwrap();
        if let Some(timer) = timers_lck.get_mut(&timer_id) {
            let value = timer.value;
            drop(timers_lck);
            self.arm_timer(timer_id, value);
        }
    }

    fn arm_timer(&self, id: TimerId, value: Duration) {
        let sender = self.sender.clone();
        let timers = self.timers.clone();
        let handle = self.runtime.handle().spawn(async move {
            let mut value = value;
            loop {
                tokio::time::sleep(value).await;
                if sender
                    .send(ThreadEvent::UIEvent(UIEvent::Timer(id)))
                    .is_err()
                {
                    break;
                }
                if let Some(interval) = timers.lock().unwrap().get(&id).and_then(|timer| {
                    if timer.interval.as_millis() == 0 && timer.interval.as_secs() == 0 {
                        None
                    } else if timer.active {
                        Some(timer.interval)
                    } else {
                        None
                    }
                }) {
                    value = interval;
                } else {
                    break;
                }
            }
        });
        self.timers.lock().unwrap().entry(id).and_modify(|timer| {
            // Abort a superseded arming so re-arming a periodic timer never
            // leaves two tasks ticking for the same `TimerId`.
            if let Some(old) = timer.handle.replace(handle.abort_handle()) {
                old.abort();
            }
            timer.active = true;
        });
    }

    fn disable_timer(&self, id: TimerId) {
        let mut timers_lck = self.timers.lock().unwrap();
        if let Some(timer) = timers_lck.get_mut(&id) {
            timer.active = false;
            if let Some(handle) = timer.handle.take() {
                handle.abort();
            }
        }
    }

    fn set_interval(&self, id: TimerId, new_val: Duration) {
        let mut timers_lck = self.timers.lock().unwrap();
        if let Some(timer) = timers_lck.get_mut(&id) {
            timer.interval = new_val;
        }
    }

    pub fn set_job_finished(&self, id: JobId) {
        self.jobs.lock().unwrap().entry(id).and_modify(|entry| {
            entry.finished.set_finished(Some(datetime::now()));
        });
    }

    pub fn set_job_success(&self, id: JobId, value: bool) {
        self.jobs.lock().unwrap().entry(id).and_modify(|entry| {
            entry.succeeded = value;
        });
    }
}

pub type JobChannel<T> = oneshot::Receiver<T>;

/// `JoinHandle` for the future that allows us to cancel the task.
#[derive(Debug)]
pub struct JoinHandle<T> {
    abort: tokio::task::AbortHandle,
    pub chan: JobChannel<T>,
    pub cancel: Arc<AtomicBool>,
    finished: FinishedTimestamp,
    pub job_id: JobId,
}

impl<T> JoinHandle<T> {
    pub fn cancel(&self) -> Option<StatusEvent> {
        let was_active = self.cancel.swap(true, Ordering::SeqCst);
        // Abort the runtime task at its next `.await` point (dropping the
        // future and releasing its resources); repeated calls are no-ops.
        self.abort.abort();
        if was_active {
            self.finished.set_finished(Some(datetime::now()));
            Some(StatusEvent::JobCanceled(self.job_id))
        } else {
            None
        }
    }

    pub fn finished(&self) -> Option<UnixTimestamp> {
        self.finished.finished()
    }

    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

impl<T> std::cmp::PartialEq<JobId> for JoinHandle<T> {
    fn eq(&self, other: &JobId) -> bool {
        self.job_id == *other
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        _ = self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use futures::future::pending;

    use super::*;

    /// Wait for the next `ThreadEvent` on the main-loop channel, mirroring
    /// how the real event loop consumes job completions.
    fn next_event(rx: &crossbeam::channel::Receiver<ThreadEvent>) -> Option<ThreadEvent> {
        rx.recv_timeout(Duration::from_secs(5)).ok()
    }

    #[test]
    fn test_spawn_async_job_completes() {
        let (sender, receiver) = crossbeam::channel::unbounded();
        let executor = JobExecutor::new(sender);
        let mut handle = executor.spawn(
            Cow::Borrowed("async test job"),
            async { 1 + 1 },
            IsAsync::Async,
        );
        let job_id = handle.job_id;
        assert!(
            matches!(next_event(&receiver), Some(ThreadEvent::JobFinished(id)) if id == job_id),
            "async job must report completion exactly once"
        );
        assert_eq!(handle.chan.try_recv().ok().flatten(), Some(2));
        let jobs = executor.jobs.lock().unwrap();
        let metadata = jobs.get(&job_id).expect("job metadata must exist");
        assert_eq!(metadata.description(), "async test job");
        assert!(metadata.finished().is_some());
        assert!(metadata.succeeded());
    }

    #[test]
    fn test_spawn_blocking_job_completes() {
        let (sender, receiver) = crossbeam::channel::unbounded();
        let executor = JobExecutor::new(sender);
        let mut handle = executor.spawn(
            Cow::Borrowed("blocking test job"),
            async {
                std::thread::sleep(Duration::from_millis(10));
                "done"
            },
            IsAsync::Blocking,
        );
        let job_id = handle.job_id;
        assert!(
            matches!(next_event(&receiver), Some(ThreadEvent::JobFinished(id)) if id == job_id),
            "blocking job must report completion exactly once"
        );
        assert_eq!(handle.chan.try_recv().ok().flatten(), Some("done"));
    }

    #[test]
    fn test_cancel_aborts_pending_job() {
        let (sender, receiver) = crossbeam::channel::unbounded();
        let executor = JobExecutor::new(sender);
        let handle = executor.spawn(
            Cow::Borrowed("never-ending test job"),
            async { pending::<i32>().await },
            IsAsync::Async,
        );
        let job_id = handle.job_id;
        // The first cancel stops the job; the `JobCanceled` status is only
        // reported by a subsequent cancel (behavior kept from the previous
        // executor).
        assert!(handle.cancel().is_none());
        assert!(handle.is_canceled());
        assert!(matches!(
            handle.cancel(),
            Some(StatusEvent::JobCanceled(id)) if id == job_id
        ));
        drop(handle);
        // The aborted job must never report completion.
        receiver
            .recv_timeout(Duration::from_millis(200))
            .unwrap_err();
    }

    #[test]
    fn test_panicking_job_is_isolated_and_reported() {
        let (sender, receiver) = crossbeam::channel::unbounded();
        let executor = JobExecutor::new(sender);
        let mut handle = executor.spawn(
            Cow::Borrowed("panicking test job"),
            async { panic!("boom") },
            IsAsync::Async,
        );
        let job_id = handle.job_id;
        assert!(
            matches!(next_event(&receiver), Some(ThreadEvent::JobFinished(id)) if id == job_id),
            "a panicking job must still be reported as finished"
        );
        handle.chan.try_recv().unwrap_err();
        assert!(!executor.jobs.lock().unwrap()[&job_id].succeeded());
    }

    #[test]
    fn test_timer_fires_and_can_be_disabled() {
        let (sender, receiver) = crossbeam::channel::unbounded();
        let executor = Arc::new(JobExecutor::new(sender));
        // Periodic timer: interval == value == 10ms.
        let timer = executor.create_timer(Duration::from_millis(10), Duration::from_millis(10));
        let timer_id = timer.id();
        let is_timer_tick = |event: ThreadEvent| matches!(event, ThreadEvent::UIEvent(UIEvent::Timer(id)) if id == timer_id);
        assert!(
            next_event(&receiver).is_some_and(is_timer_tick),
            "periodic timer must tick"
        );
        timer.disable();
        assert!(
            receiver.recv_timeout(Duration::from_millis(200)).is_err(),
            "disabled timer must not tick again"
        );
    }

    #[test]
    fn test_timer_rearm_does_not_duplicate() {
        let (sender, receiver) = crossbeam::channel::unbounded();
        let executor = Arc::new(JobExecutor::new(sender));
        // One-shot timer: zero interval fires once and stops.
        let timer = executor.create_timer(Duration::ZERO, Duration::from_millis(30));
        let timer_id = timer.id();
        timer.rearm();
        timer.rearm();
        let mut ticks = 0;
        while let Ok(event) = receiver.recv_timeout(Duration::from_millis(300)) {
            if matches!(&event, ThreadEvent::UIEvent(UIEvent::Timer(id)) if *id == timer_id) {
                ticks += 1;
            }
        }
        assert_eq!(
            ticks, 1,
            "re-arming must replace the pending tick, not duplicate it"
        );
    }
}
