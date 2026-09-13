//
// meli
//
// Copyright 2026 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

//! Fork release `v0.9.0`: <https://github.com/kylelee/hardened-meli>

use crate::version_migrations::*;

/// Fork release `v0.9.0`.
pub const V0_9_0_ID: VersionIdentifier = VersionIdentifier {
    string: "0.9.0",
    major: 0,
    minor: 9,
    patch: 0,
    pre: "",
};

/// Fork release `v0.9.0`.
#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub struct V0_9_0;

impl Version for V0_9_0 {
    fn version(&self) -> &VersionIdentifier {
        &V0_9_0_ID
    }

    fn migrations(&self) -> Vec<Box<dyn Migration + Send + Sync + 'static>> {
        vec![]
    }
}
