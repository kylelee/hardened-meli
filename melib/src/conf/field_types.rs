//
// meli
//
// Copyright 2017-  Manos Pitsidianakis
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

use serde::{
    de::{Deserialize, Deserializer},
    ser::{Serialize, Serializer},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ToggleFlag {
    #[default]
    Unset,
    InternalVal(bool),
    False,
    True,
}

impl From<bool> for ToggleFlag {
    fn from(val: bool) -> Self {
        if val {
            Self::True
        } else {
            Self::False
        }
    }
}

impl ToggleFlag {
    pub fn is_unset(&self) -> bool {
        Self::Unset == *self
    }

    pub fn is_internal(&self) -> bool {
        matches!(self, Self::InternalVal(_))
    }

    pub fn is_false(&self) -> bool {
        matches!(self, Self::False | Self::InternalVal(false))
    }

    pub fn is_true(&self) -> bool {
        matches!(self, Self::True | Self::InternalVal(true))
    }
}

impl Serialize for ToggleFlag {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Unset | Self::InternalVal(_) => serializer.serialize_none(),
            Self::False => serializer.serialize_bool(false),
            Self::True => serializer.serialize_bool(true),
        }
    }
}

impl<'de> Deserialize<'de> for ToggleFlag {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <bool>::deserialize(deserializer)?;
        Ok(s.into())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ActionFlag {
    InternalVal(bool),
    False,
    True,
    #[default]
    Ask,
}

impl From<bool> for ActionFlag {
    fn from(val: bool) -> Self {
        if val {
            Self::True
        } else {
            Self::False
        }
    }
}

impl ActionFlag {
    pub fn is_internal(&self) -> bool {
        matches!(self, Self::InternalVal(_))
    }

    pub fn is_ask(&self) -> bool {
        matches!(self, Self::Ask)
    }

    pub fn is_false(&self) -> bool {
        matches!(self, Self::False | Self::InternalVal(false))
    }

    pub fn is_true(&self) -> bool {
        matches!(self, Self::True | Self::InternalVal(true))
    }
}

impl Serialize for ActionFlag {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::InternalVal(_) => serializer.serialize_none(),
            Self::False => serializer.serialize_bool(false),
            Self::True => serializer.serialize_bool(true),
            Self::Ask => serializer.serialize_str("ask"),
        }
    }
}

impl<'de> Deserialize<'de> for ActionFlag {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = ActionFlag;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("either a boolean or the string \"ask\"")
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(if value {
                    ActionFlag::True
                } else {
                    ActionFlag::False
                })
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.eq_ignore_ascii_case("ask") {
                    Ok(ActionFlag::Ask)
                } else {
                    Err(E::invalid_value(
                        serde::de::Unexpected::Str(value),
                        &"either a boolean or the string \"ask\"",
                    ))
                }
            }
        }
        deserializer.deserialize_any(V)
    }
}
