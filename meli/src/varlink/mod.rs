//
// meli
//
// Copyright 2026 Manos Pitsidianakis <manos@pitsidianak.is>
// Fork modifications Copyright 2026 Kyle Lee <kylelee@users.noreply.github.com>
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

//! varlink IPC support
//!
//! Launch an IPC server with `meli --varlink <addr>` so external programs
//! can send meli commands and inject input over a varlink socket. See the
//! `varlink IPC` section in `meli.1` for the exposed interface methods and
//! example calls with `varlinkctl`.

pub mod handler;
pub mod interface;
pub mod server;
#[cfg(test)]
mod tests;
#[cfg(unix)]
pub mod unix;
