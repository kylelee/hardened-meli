/*
 * meli
 *
 * Copyright 2017-2018 Manos Pitsidianakis
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

use serde::{de, de::Visitor, Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Key {
    /// Backspace.
    Backspace,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page Up key.
    PageUp,
    /// Page Down key.
    PageDown,
    /// Delete key.
    Delete,
    /// Insert key.
    Insert,
    /// Function keys.
    ///
    /// Only function keys 1 through 12 are supported.
    F(u8),
    /// Normal character.
    Char(char),
    /// Alt modified character.
    Alt(char),
    /// Ctrl modified character.
    ///
    /// Note that certain keys may not be modifiable with `ctrl`, due to
    /// limitations of terminals.
    Ctrl(char),
    /// Null byte.
    Null,
    /// Esc key.
    Esc,
    Mouse(MouseEvent),
    Paste(String),
}

/// A mouse related event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MouseEvent {
    /// A mouse button was pressed.
    ///
    /// The coordinates are one-based.
    Press(MouseButton, u16, u16),
    /// A mouse button was released.
    ///
    /// The coordinates are one-based.
    Release(u16, u16),
    /// A mouse button is held over the given coordinates.
    ///
    /// The coordinates are one-based.
    Hold(u16, u16),
}

/// A mouse button.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MouseButton {
    /// The left mouse button.
    Left,
    /// The right mouse button.
    Right,
    /// The middle mouse button.
    Middle,
    /// Mouse wheel is going up.
    ///
    /// This event is typically only used with [`MouseEvent::Press`].
    WheelUp,
    /// Mouse wheel is going down.
    ///
    /// This event is typically only used with [`MouseEvent::Press`].
    WheelDown,
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::F(n) => write!(f, "F{n}"),
            Self::Char(' ') => write!(f, "Space"),
            Self::Char('\t') => write!(f, "Tab"),
            Self::Char('\n') => write!(f, "Enter"),
            Self::Char(c) => write!(f, "{c}"),
            Self::Alt(c) => write!(f, "M-{c}"),
            Self::Ctrl(c) => write!(f, "C-{c}"),
            Self::Paste(_) => write!(f, "Pasted buf"),
            Self::Null => write!(f, "Null byte"),
            Self::Esc => write!(f, "Esc"),
            Self::Backspace => write!(f, "Backspace"),
            Self::Left => write!(f, "Left"),
            Self::Right => write!(f, "Right"),
            Self::Up => write!(f, "Up"),
            Self::Down => write!(f, "Down"),
            Self::Home => write!(f, "Home"),
            Self::End => write!(f, "End"),
            Self::PageUp => write!(f, "PageUp"),
            Self::PageDown => write!(f, "PageDown"),
            Self::Delete => write!(f, "Delete"),
            Self::Insert => write!(f, "Insert"),
            Self::Mouse(_) => write!(f, "Mouse"),
        }
    }
}

impl Key {
    /// Status-bar hint form: placeholder keys — arrows, `Esc`, `F`-keys,
    /// modifier combinations, `Enter`/`Tab`/`Space` — render inside angle
    /// brackets (`<Up>`, `<Esc>`, `<C-c>`) so they read as key *descriptions*
    /// rather than literal text, while plain character keys (`j`, `k`, `q`,
    /// …) stay bare exactly as they are typed.
    pub fn hint_display(&self) -> String {
        match self {
            Self::Char(' ') => "<Space>".to_string(),
            Self::Char('\t') => "<Tab>".to_string(),
            Self::Char('\n') => "<Enter>".to_string(),
            Self::Char(c) => c.to_string(),
            _ => format!("<{self}>"),
        }
    }
}

impl<'a> From<&'a String> for Key {
    fn from(v: &'a String) -> Self {
        Self::Paste(v.to_string())
    }
}

impl PartialEq<Key> for &Key {
    fn eq(&self, other: &Key) -> bool {
        **self == *other
    }
}

/// Parse a single key token from its configuration string form, e.g.
/// `"Up"`, `"F5"`, `"C-c"`, `"M-x"`, `"Esc"`, `"Enter"` or a single
/// character.
///
/// This is the parsing backend shared by [`Key`]'s and
/// [`ShortcutKeys`]'s `Deserialize` implementations.
pub fn parse_key(s: &str) -> Result<Key, String> {
    match s {
        "Backspace" | "backspace" => Ok(Key::Backspace),
        "Left" | "left" => Ok(Key::Left),
        "Right" | "right" => Ok(Key::Right),
        "Up" | "up" => Ok(Key::Up),
        "Down" | "down" => Ok(Key::Down),
        "Home" | "home" => Ok(Key::Home),
        "End" | "end" => Ok(Key::End),
        "PageUp" | "pageup" => Ok(Key::PageUp),
        "PageDown" | "pagedown" => Ok(Key::PageDown),
        "Delete" | "delete" => Ok(Key::Delete),
        "Insert" | "insert" => Ok(Key::Insert),
        "Enter" | "enter" => Ok(Key::Char('\n')),
        "Tab" | "tab" => Ok(Key::Char('\t')),
        "Esc" | "esc" => Ok(Key::Esc),
        s if s.len() == 1 => Ok(Key::Char(s.chars().next().unwrap())),
        s if s.starts_with('F') && (s.len() == 2 || s.len() == 3) => {
            use std::str::FromStr;

            if let Ok(n) = u8::from_str(&s[1..]) {
                if (1..=12).contains(&n) {
                    return Ok(Key::F(n));
                }
            }
            Err(format!(
                "`{}` should be a number 1 <= n <= 12 instead.",
                &s[1..]
            ))
        }
        s if s.starts_with("M-") && s.len() == 3 => {
            let c = s.as_bytes()[2] as char;

            if c.is_lowercase() || c.is_numeric() {
                return Ok(Key::Alt(c));
            }

            Err(format!(
                "`{}` should be a lowercase and alphanumeric character instead.",
                &s[2..]
            ))
        }
        s if s.starts_with("C-") && s.len() == 3 => {
            let c = s.as_bytes()[2] as char;

            if c.is_lowercase() || c.is_numeric() {
                return Ok(Key::Ctrl(c));
            }

            Err(format!(
                "`{}` should be a lowercase and alphanumeric character instead.",
                &s[2..]
            ))
        }
        _ => Err(format!(
            "Cannot derive shortcut from `{s}`. Please consult the manual for valid key inputs."
        )),
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct KeyVisitor;

        impl Visitor<'_> for KeyVisitor {
            type Value = Key;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter
                    .write_str("a valid key value. Please consult the manual for valid key inputs.")
            }

            fn visit_char<E>(self, value: char) -> Result<Key, E>
            where
                E: de::Error,
            {
                Ok(Key::Char(value))
            }

            fn visit_str<E>(self, value: &str) -> Result<Key, E>
            where
                E: de::Error,
            {
                parse_key(value).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_any(KeyVisitor)
    }
}

impl Serialize for Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Backspace => serializer.serialize_str("Backspace"),
            Self::Left => serializer.serialize_str("Left"),
            Self::Right => serializer.serialize_str("Right"),
            Self::Up => serializer.serialize_str("Up"),
            Self::Down => serializer.serialize_str("Down"),
            Self::Home => serializer.serialize_str("Home"),
            Self::End => serializer.serialize_str("End"),
            Self::PageUp => serializer.serialize_str("PageUp"),
            Self::PageDown => serializer.serialize_str("PageDown"),
            Self::Delete => serializer.serialize_str("Delete"),
            Self::Insert => serializer.serialize_str("Insert"),
            Self::Esc => serializer.serialize_str("Esc"),
            Self::Char('\n') => serializer.serialize_str("Enter"),
            Self::Char('\t') => serializer.serialize_str("Tab"),
            Self::Char(c) => serializer.serialize_char(*c),
            Self::F(n) => serializer.serialize_str(&format!("F{n}")),
            Self::Alt(c) => serializer.serialize_str(&format!("M-{c}")),
            Self::Ctrl(c) => serializer.serialize_str(&format!("C-{c}")),
            Self::Null => serializer.serialize_str("Null"),
            Self::Mouse(mev) => mev.serialize(serializer),
            Self::Paste(s) => serializer.serialize_str(s),
        }
    }
}

/// Up to two key bindings for one shortcut, comma-separated in
/// configuration (`"Up,k"`); a bare single key (`"Up"` or a single
/// character) stays accepted and is equivalent to a one-element
/// binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutKeys(pub SmallVec<[Key; 2]>);

impl ShortcutKeys {
    /// A single-key binding.
    pub fn single(k: Key) -> Self {
        Self(smallvec::smallvec![k])
    }

    /// A two-key binding; either key triggers the shortcut.
    pub fn double(a: Key, b: Key) -> Self {
        Self(smallvec::smallvec![a, b])
    }

    /// Returns `true` if `k` is one of the bound keys.
    pub fn contains(&self, k: &Key) -> bool {
        self.0.iter().any(|bound| bound == k)
    }

    /// The configuration string form of a single key (the inverse of
    /// [`parse_key`]).
    fn key_to_config_string(k: &Key) -> String {
        match k {
            // `Key`'s `Display` renders this as `Space`, which does not
            // parse back; the literal character does.
            Key::Char(' ') => " ".to_string(),
            k => k.to_string(),
        }
    }
}

impl From<Key> for ShortcutKeys {
    fn from(k: Key) -> Self {
        Self::single(k)
    }
}

impl std::fmt::Display for ShortcutKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // The joined text has to go through `Formatter::pad`: width and
        // alignment specs (e.g. the help overlay's right-aligned binding
        // column) only apply if the `Display` impl calls `pad`, while
        // `write_str` writes verbatim. Building one `String` and padding it
        // replaces the previous `collect::<Vec<String>>().join("/")`, which
        // allocated a `String` per key plus the vector plus the result on
        // every call (the status-bar hints render one `Display` per hint per
        // frame).
        let mut joined = String::with_capacity(self.0.len() * 4);
        for (i, k) in self.0.iter().enumerate() {
            if i > 0 {
                joined.push('/');
            }
            std::fmt::Write::write_fmt(&mut joined, format_args!("{k}"))?;
        }
        f.pad(&joined)
    }
}

impl ShortcutKeys {
    /// [`Self`] in status-bar hint form: each key via [`Key::hint_display`]
    /// (placeholder keys in angle brackets, plain characters bare), joined
    /// with `/` — e.g. `<Up>/k`, `<Esc>/q`.
    pub fn hint_display(&self) -> String {
        let mut joined = String::with_capacity(self.0.len() * 4);
        for (i, k) in self.0.iter().enumerate() {
            if i > 0 {
                joined.push('/');
            }
            joined.push_str(&k.hint_display());
        }
        joined
    }
}

impl<'de> Deserialize<'de> for ShortcutKeys {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ShortcutKeysVisitor;

        impl Visitor<'_> for ShortcutKeysVisitor {
            type Value = ShortcutKeys;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter
                    .write_str("a key or at most two comma-separated keys, e.g. \"Up\" or \"Up,k\"")
            }

            fn visit_char<E>(self, value: char) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(ShortcutKeys::single(Key::Char(value)))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                // A lone comma (or any value that is itself a valid single
                // key) is one binding, not a separator: try the whole value
                // first so `Key::Char(',')` round-trips through `Serialize`
                // instead of splitting into two empty segments.
                if let Ok(k) = parse_key(value) {
                    return Ok(ShortcutKeys::single(k));
                }
                // Segments are deliberately NOT trimmed: trimming would
                // break bindings whose key is the space character,
                // which is a valid (and default) binding for e.g.
                // `listing.toggle_mailbox_collapse`.
                let keys = value
                    .split(',')
                    .map(parse_key)
                    .collect::<Result<SmallVec<[Key; 2]>, String>>()
                    .map_err(de::Error::custom)?;
                if keys.len() > 2 {
                    return Err(de::Error::custom(
                        "at most two comma-separated keys are allowed",
                    ));
                }
                Ok(ShortcutKeys(keys))
            }
        }

        deserializer.deserialize_any(ShortcutKeysVisitor)
    }
}

impl Serialize for ShortcutKeys {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let parts = self
            .0
            .iter()
            .map(Self::key_to_config_string)
            .collect::<Vec<_>>();
        // The wire form is comma-separated, so a comma key is only
        // representable on its own. A pair containing one would serialize
        // to a string this deserializer cannot parse back (silent config
        // corruption); reject it instead of emitting it.
        if parts.len() > 1 && parts.iter().any(|p| p.contains(',')) {
            return Err(serde::ser::Error::custom(
                "a comma key cannot be serialized together with a second binding key",
            ));
        }
        serializer.serialize_str(&parts.join(","))
    }
}

#[test]
fn test_key_serde() {
    use serde_test2::{
        assert_de_tokens, assert_de_tokens_error, assert_ser_tokens, assert_tokens, Token,
    };

    assert_tokens(&Key::Backspace, &[Token::Str("Backspace")]);
    assert_tokens(&Key::Left, &[Token::Str("Left")]);
    assert_tokens(&Key::Right, &[Token::Str("Right")]);
    assert_tokens(&Key::Up, &[Token::Str("Up")]);
    assert_tokens(&Key::Down, &[Token::Str("Down")]);
    assert_tokens(&Key::Home, &[Token::Str("Home")]);
    assert_tokens(&Key::End, &[Token::Str("End")]);
    assert_tokens(&Key::PageUp, &[Token::Str("PageUp")]);
    assert_tokens(&Key::PageDown, &[Token::Str("PageDown")]);
    assert_tokens(&Key::Delete, &[Token::Str("Delete")]);
    assert_tokens(&Key::Insert, &[Token::Str("Insert")]);
    assert_tokens(&Key::Char('\n'), &[Token::Str("Enter")]);
    assert_tokens(&Key::Esc, &[Token::Str("Esc")]);
    assert_tokens(&Key::Char('\t'), &[Token::Str("Tab")]);
    assert_tokens(&Key::Ctrl('a'), &[Token::Str("C-a")]);
    assert_tokens(&Key::Ctrl('1'), &[Token::Str("C-1")]);
    assert_tokens(&Key::Alt('a'), &[Token::Str("M-a")]);
    assert_tokens(&Key::F(1), &[Token::Str("F1")]);
    assert_tokens(&Key::F(12), &[Token::Str("F12")]);

    // Key::Char deserialises from both string and char, but only serialises to char

    // Round-trip
    assert_tokens(&Key::Char('k'), &[Token::Char('k')]);
    assert_tokens(&Key::Char('1'), &[Token::Char('1')]);

    // One-way
    assert_de_tokens(&Key::Char('1'), &[Token::Str("1")]);
    assert_ser_tokens(&Key::Char('1'), &[Token::Char('1')]);

    assert_de_tokens_error::<Key>(
        &[Token::Str("C-V")],
        "`V` should be a lowercase and alphanumeric character instead.",
    );
    assert_de_tokens_error::<Key>(
        &[Token::Str("M-V")],
        "`V` should be a lowercase and alphanumeric character instead.",
    );
    assert_de_tokens_error::<Key>(
        &[Token::Str("F13")],
        "`13` should be a number 1 <= n <= 12 instead.",
    );
    assert_de_tokens_error::<Key>(
        &[Token::Str("Fc")],
        "`c` should be a number 1 <= n <= 12 instead.",
    );
    assert_de_tokens_error::<Key>(
        &[Token::Str("adsfsf")],
        "Cannot derive shortcut from `adsfsf`. Please consult the manual for valid key inputs.",
    );
}

#[test]
fn test_shortcut_keys_serde() {
    use serde_test2::{
        assert_de_tokens, assert_de_tokens_error, assert_ser_tokens_error, assert_tokens, Token,
    };

    // Round-trip: a comma-separated pair parses to a double binding and
    // serializes back to the same comma-separated form.
    assert_tokens(
        &ShortcutKeys::double(Key::Up, Key::Char('k')),
        &[Token::Str("Up,k")],
    );
    assert_tokens(
        &ShortcutKeys::double(Key::Esc, Key::Char('q')),
        &[Token::Str("Esc,q")],
    );
    // A bare single key value stays a single binding.
    assert_tokens(&ShortcutKeys::single(Key::Up), &[Token::Str("Up")]);
    // A bare char token (e.g. TOML `quit = 'q'`) is a single binding.
    assert_de_tokens(&ShortcutKeys::single(Key::Char('k')), &[Token::Char('k')]);
    // The space key must round-trip through its literal character.
    assert_tokens(&ShortcutKeys::single(Key::Char(' ')), &[Token::Str(" ")]);
    assert_tokens(
        &ShortcutKeys::double(Key::Up, Key::Char(' ')),
        &[Token::Str("Up, ")],
    );
    // A comma *is* a valid key and serializes to ","; parsing must treat the
    // whole value as one binding first, or the round trip splits it into two
    // empty segments.
    assert_tokens(&ShortcutKeys::single(Key::Char(',')), &[Token::Str(",")]);
    // A pair containing a comma key is not representable in the
    // comma-separated wire form: serializing it is an error rather than a
    // string this deserializer would reject.
    assert_ser_tokens_error(
        &ShortcutKeys::double(Key::Up, Key::Char(',')),
        &[],
        "a comma key cannot be serialized together with a second binding key",
    );
    // More than two keys are rejected instead of truncated.
    assert_de_tokens_error::<ShortcutKeys>(
        &[Token::Str("Up,k,j")],
        "at most two comma-separated keys are allowed",
    );
    // An invalid segment reports the segment verbatim.
    assert_de_tokens_error::<ShortcutKeys>(
        &[Token::Str("Up,F13")],
        "`13` should be a number 1 <= n <= 12 instead.",
    );
}

/// `Key::hint_display` / `ShortcutKeys::hint_display`: the status-bar hint
/// form wraps placeholder keys in angle brackets so they read as key
/// descriptions (`<Up>`, `<Esc>`, `<C-c>`), while plain character keys stay
/// bare exactly as they are typed (`j`, `k`, `q`, `?`).
#[test]
fn test_hint_display() {
    assert_eq!(Key::Up.hint_display(), "<Up>");
    assert_eq!(Key::Down.hint_display(), "<Down>");
    assert_eq!(Key::Left.hint_display(), "<Left>");
    assert_eq!(Key::Right.hint_display(), "<Right>");
    assert_eq!(Key::Esc.hint_display(), "<Esc>");
    assert_eq!(Key::Home.hint_display(), "<Home>");
    assert_eq!(Key::End.hint_display(), "<End>");
    assert_eq!(Key::PageUp.hint_display(), "<PageUp>");
    assert_eq!(Key::PageDown.hint_display(), "<PageDown>");
    assert_eq!(Key::Backspace.hint_display(), "<Backspace>");
    assert_eq!(Key::Delete.hint_display(), "<Delete>");
    assert_eq!(Key::Insert.hint_display(), "<Insert>");
    assert_eq!(Key::F(5).hint_display(), "<F5>");
    assert_eq!(Key::Ctrl('c').hint_display(), "<C-c>");
    assert_eq!(Key::Alt('x').hint_display(), "<M-x>");
    // Whitespace control keys have word forms and count as placeholders.
    assert_eq!(Key::Char(' ').hint_display(), "<Space>");
    assert_eq!(Key::Char('\t').hint_display(), "<Tab>");
    assert_eq!(Key::Char('\n').hint_display(), "<Enter>");
    // Plain character keys stay bare.
    for c in ['j', 'k', 'h', 'l', 'q', '?', 'g'] {
        assert_eq!(Key::Char(c).hint_display(), c.to_string());
    }
    // Joined bindings keep the `/` separator.
    assert_eq!(
        ShortcutKeys::double(Key::Up, Key::Char('k')).hint_display(),
        "<Up>/k"
    );
    assert_eq!(
        ShortcutKeys::double(Key::Esc, Key::Char('q')).hint_display(),
        "<Esc>/q"
    );
    assert_eq!(ShortcutKeys::single(Key::Char('j')).hint_display(), "j");
}
