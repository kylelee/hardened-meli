/*
 * meli
 *
 * Copyright 2019 Manos Pitsidianakis
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

use std::{
    fs::OpenOptions,
    io::{Read, Write},
    sync::{Arc, Mutex},
};

thread_local!(static CMD_HISTORY_FILE: Arc<Mutex<Option<std::fs::File>>> = Arc::new(Mutex::new(open_history_file())));

/// Open the persistent command history file.
///
/// History is a convenience: an unavailable data directory, an unwritable
/// path, or a poisoned lock must never abort the process, so failures are
/// logged and reported as `None`.
fn open_history_file() -> Option<std::fs::File> {
    let data_dir = match xdg::BaseDirectories::with_prefix("meli") {
        Ok(d) => d,
        Err(err) => {
            melib::log::error!("Could not locate data directory for command history: {err}");
            return None;
        }
    };
    let path = match data_dir.place_data_file("cmd_history") {
        Ok(p) => p,
        Err(err) => {
            melib::log::error!("Could not locate command history file: {err}");
            return None;
        }
    };
    match OpenOptions::new()
        .append(true) /* writes will append to a file instead of overwriting previous contents */
        .create(true) /* a new file will be created if the file does not yet already exist. */
        .read(true)
        .open(&path)
    {
        Ok(file) => Some(file),
        Err(err) => {
            melib::log::error!(
                "Could not open command history file `{}`: {err}",
                path.display()
            );
            None
        }
    }
}

pub fn log_cmd(mut cmd: String) {
    CMD_HISTORY_FILE.with(|f| {
        let Ok(mut guard) = f.lock() else {
            return;
        };
        let Some(file) = guard.as_mut() else {
            return;
        };
        cmd.push('\n');
        if let Err(err) = file.write_all(cmd.as_bytes()) {
            melib::log::error!("Could not write to command history: {err}");
        }
    });
}

pub fn old_cmd_history() -> Vec<String> {
    let mut ret = Vec::new();
    CMD_HISTORY_FILE.with(|f| {
        let Ok(mut guard) = f.lock() else {
            return;
        };
        let Some(file) = guard.as_mut() else {
            return;
        };
        let mut old_history = String::new();
        if let Err(err) = file.read_to_string(&mut old_history) {
            // Non-UTF-8 or otherwise unreadable history must not abort when
            // the command bar is opened.
            melib::log::error!("Could not read command history: {err}");
            return;
        }
        ret.extend(old_history.lines().map(|s| s.to_string()));
    });
    ret
}
