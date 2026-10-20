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

//! CLI smoke test: the `tools managesieve-shell` subcommand must parse
//! `--help` (upstream port `4cc43b035`).

#[test]
fn tools_managesieve_shell_help() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_meli"))
        .args(["tools", "managesieve-shell", "--help"])
        .output()
        .expect("meli binary must run");
    assert!(
        output.status.success(),
        "--help must exit successfully: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("managesieve-shell") && stdout.contains("CONFIG_TOML_ACCOUNT_NAME"),
        "--help must document the subcommand: {stdout}"
    );
}
