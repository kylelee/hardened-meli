//
// meli
//
// Copyright 2026 Kyle Lee
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

//! Process-global `tracing` configuration, installed once at startup by
//! [`init_log`].
//!
//! - Formatting: `tracing-subscriber`'s pretty formatter, ANSI disabled
//!   (the output is a log file).
//! - Destination: `tracing-appender`'s hourly rolling appender writing
//!   `./log/meli.<YYYY-MM-DD-HH>` files (one per UTC hour), kept private
//!   (`0600` on unix).
//! - Retention: log files older than [`RETENTION`] (7 days) are deleted,
//!   once at startup and then every [`RETENTION_CHECK_INTERVAL`] (1 hour)
//!   by a background thread.
//! - Level: `DEBUG` in debug builds, `ERROR` in release builds (release
//!   builds additionally compile everything below `ERROR` out entirely via
//!   `tracing`'s `release_max_level_error` feature on the `melib`
//!   dependency).
//! - Setting `MELI_DEBUG_STDERR` in the environment duplicates every
//!   formatted line on stderr (e.g. `MELI_DEBUG_STDERR=yes meli 2>
//!   trace.log`).

use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Once},
    thread,
    time::{Duration, SystemTime},
};

use tracing::level_filters::LevelFilter;
use tracing_appender::rolling::RollingFileAppender;
use tracing_subscriber::fmt::writer::MakeWriter;

/// Log directory (relative to the working directory meli was started in).
pub const LOG_DIR: &str = "log";
/// Log file name prefix; the hourly appender appends `.<YYYY-MM-DD-HH>`.
pub const LOG_FILE_PREFIX: &str = "meli";
/// How long log files are kept before the retention pass deletes them.
pub const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How often the retention pass runs.
pub const RETENTION_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Initialize all `tracing` configuration at startup.
///
/// Installs the global subscriber (pretty format, hourly rolling files
/// under `./log/`, `DEBUG` level in debug builds and `ERROR` in release
/// builds, stderr duplication with `MELI_DEBUG_STDERR`) and starts the
/// 7-day retention cleanup.
///
/// Idempotent; safe to call again (later calls are no-ops). Under the test
/// harness (`cfg!(test)`) this does nothing, so unit tests never write to
/// `./log/` nor install a process-global subscriber.
pub fn init_log() {
    if cfg!(test) {
        return;
    }

    static INIT: Once = Once::new();

    INIT.call_once(|| {
        let level = if cfg!(debug_assertions) {
            LevelFilter::DEBUG
        } else {
            LevelFilter::ERROR
        };
        let also_stderr = std::env::var_os("MELI_DEBUG_STDERR").is_some();
        let appender = rolling_appender();
        let subscriber = tracing_subscriber::fmt()
            .pretty()
            .with_ansi(false)
            .with_max_level(level)
            .with_writer(TeeWriter::new(appender, also_stderr))
            .finish();
        if tracing::subscriber::set_global_default(subscriber).is_ok() {
            // Tighten file permissions and sweep files left over from
            // previous runs first, then keep sweeping hourly.
            enforce_private_log_files(Path::new(LOG_DIR), LOG_FILE_PREFIX);
            _ = run_retention_once(Path::new(LOG_DIR), LOG_FILE_PREFIX, RETENTION);
            spawn_retention_thread();
        }
    });
}

/// The directory holding this run's log files (see [`LOG_DIR`]).
pub fn log_dir() -> PathBuf {
    PathBuf::from(LOG_DIR)
}

fn rolling_appender() -> RollingFileAppender {
    tracing_appender::rolling::hourly(LOG_DIR, LOG_FILE_PREFIX)
}

/// Delete `<prefix>.*` files under `dir` whose modification time is older
/// than `keep`. Returns how many files were removed.
fn run_retention_once(dir: &Path, prefix: &str, keep: Duration) -> usize {
    let Some(cutoff) = SystemTime::now().checked_sub(keep) else {
        return 0;
    };
    let file_prefix = format!("{prefix}.");
    let mut removed = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let is_ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(&file_prefix));
        if !is_ours {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified < cutoff && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Log files hold account names, subjects and errors; keep them private to
/// the user (mode `0600` on unix), tightening idempotently like every other
/// file meli creates — including logs left looser by an older meli.
#[cfg(unix)]
fn enforce_private_log_files(dir: &Path, prefix: &str) {
    use std::os::unix::fs::PermissionsExt;

    let file_prefix = format!("{prefix}.");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let is_ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(&file_prefix));
        if !is_ours {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let mut permissions = metadata.permissions();
        if permissions.mode() & 0o777 != 0o600 {
            permissions.set_mode(0o600);
            let _ = std::fs::set_permissions(entry.path(), permissions);
        }
    }
}

#[cfg(not(unix))]
fn enforce_private_log_files(_dir: &Path, _prefix: &str) {}

fn spawn_retention_thread() {
    let dir = log_dir();
    let _ = thread::Builder::new()
        .name("log-retention".into())
        .spawn(move || loop {
            thread::sleep(RETENTION_CHECK_INTERVAL);
            enforce_private_log_files(&dir, LOG_FILE_PREFIX);
            _ = run_retention_once(&dir, LOG_FILE_PREFIX, RETENTION);
        });
}

/// [`MakeWriter`] writing every formatted line to the rolling log file and
/// (optionally) duplicating it on stderr.
#[derive(Clone)]
struct TeeWriter {
    appender: Arc<Mutex<RollingFileAppender>>,
    also_stderr: bool,
}

impl TeeWriter {
    fn new(appender: RollingFileAppender, also_stderr: bool) -> Self {
        Self {
            appender: Arc::new(Mutex::new(appender)),
            also_stderr,
        }
    }
}

impl io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.appender.lock().map_err(poisoned)?.write(buf)?;
        if self.also_stderr {
            // Stderr duplication is best-effort.
            let _ = io::stderr().write_all(buf);
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.appender.lock().map_err(poisoned)?.flush()?;
        if self.also_stderr {
            let _ = io::stderr().flush();
        }
        Ok(())
    }
}

fn poisoned<T>(_: T) -> io::Error {
    io::Error::other("log writer mutex poisoned")
}

impl<'a> MakeWriter<'a> for TeeWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_modified(path: &Path, when: SystemTime) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn retention_removes_only_old_prefixed_files() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let dir = temp_dir.path();
        let fresh = dir.join("meli.2099-01-01-00");
        let old = dir.join("meli.2020-01-01-00");
        let unrelated = dir.join("unrelated.log");
        std::fs::write(&fresh, b"x").unwrap();
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(&unrelated, b"x").unwrap();

        let now = SystemTime::now();
        set_modified(&fresh, now);
        set_modified(&old, now - RETENTION - Duration::from_secs(3600));
        set_modified(&unrelated, now - RETENTION - Duration::from_secs(3600));

        let removed = run_retention_once(dir, LOG_FILE_PREFIX, RETENTION);

        assert_eq!(removed, 1);
        assert!(!old.exists(), "expired log file must be deleted");
        assert!(fresh.exists(), "fresh log file must be kept");
        assert!(
            unrelated.exists(),
            "files not created by meli must never be touched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn log_files_are_tightened_to_0600() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let dir = temp_dir.path();
        let log_file = dir.join("meli.2020-01-01-00");
        let unrelated = dir.join("unrelated.log");
        std::fs::write(&log_file, b"x").unwrap();
        std::fs::write(&unrelated, b"x").unwrap();
        let mut permissions = std::fs::metadata(&log_file).unwrap().permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(&log_file, permissions).unwrap();

        enforce_private_log_files(dir, LOG_FILE_PREFIX);

        let mode = std::fs::metadata(&log_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "log file mode was {mode:o}");
        let mode = std::fs::metadata(&unrelated).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "unrelated files must not be touched");
    }

    #[test]
    fn retention_on_missing_directory_is_a_no_op() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        assert_eq!(
            run_retention_once(&temp_dir.path().join("nope"), LOG_FILE_PREFIX, RETENTION),
            0
        );
    }

    /// A scoped (`with_default`) subscriber writing through the same
    /// pretty/rolling pipeline as `init_log` must record enabled events in
    /// the rolling file and drop filtered ones.
    #[test]
    fn pretty_events_land_in_the_rolling_file_with_level_filter() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let dir = temp_dir.path().join("log");

        let subscriber = tracing_subscriber::fmt()
            .pretty()
            .with_ansi(false)
            .with_max_level(LevelFilter::DEBUG)
            .with_writer(TeeWriter::new(
                tracing_appender::rolling::hourly(&dir, LOG_FILE_PREFIX),
                false,
            ))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("e2e error marker LOG-E2E-ERR-1");
            tracing::trace!("e2e trace marker LOG-E2E-TRC-1");
        });

        let log_file = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("meli."))
            })
            .expect("a rolling log file must exist");
        let contents = std::fs::read_to_string(log_file.path()).unwrap();
        assert!(
            contents.contains("ERROR") && contents.contains("LOG-E2E-ERR-1"),
            "error event missing from the rolling log file; contents were:\n{contents}"
        );
        assert!(
            !contents.contains("LOG-E2E-TRC-1"),
            "trace event leaked past the DEBUG level; contents were:\n{contents}"
        );
    }
}
