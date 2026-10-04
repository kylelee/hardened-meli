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

use std::{
    borrow::Cow,
    fs,
    fs::OpenOptions,
    io::{Read, Write},
    os::{fd::OwnedFd, unix::fs::PermissionsExt},
    path::{Path, PathBuf},
};

use melib::{
    error::*,
    text::{TextProcessing, Truncate},
    uuid::Uuid,
    ShellExpandTrait,
};

/// Temporary file that can optionally cleaned up when it is dropped.
#[derive(Debug)]
pub struct File {
    /// File's path.
    path: PathBuf,
    /// Delete file when it is dropped.
    delete_on_drop: bool,
}

impl Drop for File {
    fn drop(&mut self) {
        if self.delete_on_drop {
            let _ = std::fs::remove_file(self.path());
        }
    }
}

impl File {
    /// Open as a standard library file type.
    pub fn as_std_file(&self) -> Result<std::fs::File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.path)
            .chain_err_summary(|| format!("Could not create/open path {}", self.path.display()))
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Convenience method to read `File` to `String`.
    pub fn read_to_string(&self) -> Result<String> {
        fn inner(path: &Path) -> Result<String> {
            let mut buf = Vec::new();
            let mut f = fs::File::open(path)?;
            f.read_to_end(&mut buf)?;
            Ok(String::from_utf8(buf)?)
        }
        inner(&self.path).chain_err_summary(|| format!("Can't read {}", self.path.display()))
    }

    /// Returned `File` will be deleted when dropped if `delete_on_drop` is set,
    /// so make sure to add it on `context.temp_files` to reap it later.
    pub fn create_temp_file(
        bytes: &[u8],
        filename: Option<&str>,
        mut path: Option<&mut PathBuf>,
        extension: Option<&str>,
        delete_on_drop: bool,
    ) -> Result<Self> {
        let filename_value: Option<Cow<'_, str>> = filename.map(|f| {
            let mut f = Cow::Borrowed(f);
            sanitize_filename(&mut f);
            // An arbitrarily long hint must not reach the retry loop:
            // pre-truncate it to a bounded component (CVE-2003-0376
            // regression, see [`FILENAME_COMPONENT_MAX_BYTES`]).
            cap_filename_component_bytes(&mut f);
            f
        });
        let mut filename: Option<&str> = filename_value.as_deref();
        // A mail-controlled name that sanitizes down to a special relative
        // component (`""`, `.`, `..`) is not a usable file name: use the
        // generated one instead of pushing it into the target directory.
        if filename.is_some_and(|f| matches!(f, "" | "." | "..")) {
            filename = None;
        }

        loop {
            let mut dir = std::env::temp_dir();
            let path = if let Some(ref mut p) = path {
                if p.try_exists().unwrap_or_default() && p.is_dir() {
                    if let Some(filename) = filename {
                        p.push(filename);
                        'exists: while p.try_exists().unwrap_or_default() {
                            for i in 0..u8::MAX {
                                p.pop();
                                p.push(format!("{filename}_{i}"));
                                if p.try_exists().unwrap_or_default() {
                                    break 'exists;
                                }
                            }
                            while p.try_exists().unwrap_or_default() {
                                p.pop();
                                p.push(format!("{filename}_{}", Uuid::new_v4().as_simple()));
                            }
                        }
                    } else {
                        let u = Uuid::new_v4();
                        p.push(u.as_simple().to_string());
                    }
                }
                p
            } else {
                dir.push("meli");
                // This staging directory sits under the shared, world-writable
                // temp root. Created with the process umask it is usually 0o755,
                // so every other local user can list the attachment names a
                // victim lands here — the information-leak half of
                // CVE-2002-1210 (issue #59). Create it owner-only, and
                // best-effort re-tighten a directory that already exists;
                // ignoring a failure is deliberate, we may not own it.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;

                    if dir.try_exists().unwrap_or_default() {
                        let _ =
                            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
                    } else {
                        std::fs::DirBuilder::new()
                            .recursive(true)
                            .mode(0o700)
                            .create(&dir)?;
                    }
                }
                #[cfg(not(unix))]
                {
                    std::fs::DirBuilder::new().recursive(true).create(&dir)?;
                }
                if let Some(filename) = filename {
                    // CVE-2002-1210 (issue #59): the mailcap `%s` landing and
                    // the open-with default application must not materialize a
                    // sender-predictable path. The sanitized hint gets a random
                    // UUID v4 infix before its extension, and the existing
                    // collision retry loop runs on top of that name.
                    let randomized = randomized_temp_component(filename);
                    dir.push(randomized.as_str());
                    'exists: while dir.try_exists().unwrap_or_default() {
                        for i in 0..u8::MAX {
                            dir.pop();
                            dir.push(format!("{randomized}_{i}"));
                            if dir.try_exists().unwrap_or_default() {
                                break 'exists;
                            }
                        }
                        while dir.try_exists().unwrap_or_default() {
                            dir.pop();
                            dir.push(format!("{randomized}_{}", Uuid::new_v4().as_simple()));
                        }
                    }
                } else {
                    let u = Uuid::new_v4();
                    dir.push(u.as_simple().to_string());
                }
                &mut dir
            };
            if let Some(ext) = extension {
                path.set_extension(ext);
            }
            fn inner(path: &Path, bytes: &[u8], delete_on_drop: bool) -> Result<File> {
                let path = path.expand();
                let mut f = std::fs::File::options()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                let metadata = f.metadata()?;
                let mut permissions = metadata.permissions();

                permissions.set_mode(0o600); // Read/write for owner only.
                f.set_permissions(permissions)?;

                f.write_all(bytes)?;
                f.flush()?;
                Ok(File {
                    path,
                    delete_on_drop,
                })
            }
            match (inner(path, bytes, delete_on_drop), filename) {
                (Err(err), Some(ref mut val))
                    if matches!(
                        err.kind,
                        ErrorKind::OSError(Errno::ENAMETOOLONG | Errno::EEXIST)
                    ) && val.grapheme_len() > 1 =>
                {
                    val.truncate_at_boundary(val.grapheme_len().saturating_sub(1));
                    filename = Some(val);
                }
                (Err(err), _) => {
                    return Err(err).chain_err_summary(|| {
                        format!("Could not create file at path {}", path.display())
                    })
                }
                (ok @ Ok(_), _) => return ok,
            }
        }
    }
}

pub fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    nix::unistd::pipe().map_err(|err| {
        Error::new("Could not create pipe")
            .set_source(Some(
                (Box::new(err) as Box<dyn std::error::Error + Send + Sync + 'static>).into(),
            ))
            .set_kind(ErrorKind::Platform)
    })
}

/// Remove path separators (`/` and `\`) from filename, replacing them
/// with `_`.
///
/// Both separators are stripped on every platform, not only the native
/// [`std::path::MAIN_SEPARATOR`]: a traversal spelling must not survive
/// sanitization on any host, because the non-native separator is live on
/// the other family (Windows path APIs accept `/`; backslash spellings
/// still name SMB/ZIP-derived paths elsewhere) — the CWE-35 path
/// traversal class of CVE-2025-47176 (`'.../...//'` in Microsoft
/// Outlook).
#[inline(always)]
pub fn sanitize_separator(value: &mut Cow<'_, str>) {
    if value.contains('/') || value.contains('\\') {
        *value = Cow::Owned(value.replace(['/', '\\'], "_"));
    };
}

/// Longest single-component filename the mail-controlled sinks use.
///
/// Chosen under the mainstream filesystem `NAME_MAX` of 255 bytes,
/// with headroom for the `_<n>`/`_<uuid>` collision suffixes
/// [`File::create_temp_file`] appends and for an extension.
///
/// CVE-2003-0376 regression (issue #47): Eudora 5.2.1 crashed — and
/// stayed crashed — on an `Attachment Converted` argument that piled
/// up `.` characters past a fixed-size buffer. meli's equivalent
/// surface is the mail-controlled attachment-name hint flowing into
/// temp-file materialization: the per-grapheme `ENAMETOOLONG` retry
/// in [`File::create_temp_file`] costs O(len²) work on a hostile
/// name, so one over-long filename in one mail could freeze the
/// client for minutes — the availability face of this CVE. Every
/// mail-controlled name sink now pre-truncates on a UTF-8 character
/// boundary; the per-grapheme fallback remains only for exotic
/// filesystems whose per-component limit is smaller still.
///
/// Public so regression corpora can assert the exact budget (see
/// `cve/src/CVE-2003-0376.rs`).
pub const FILENAME_COMPONENT_MAX_BYTES: usize = 192;

/// Truncate `value` to at most [`FILENAME_COMPONENT_MAX_BYTES`] bytes,
/// cutting on a UTF-8 character boundary (never through a multi-byte
/// character).
fn cap_filename_component_bytes(value: &mut Cow<'_, str>) {
    if value.len() > FILENAME_COMPONENT_MAX_BYTES {
        let mut end = FILENAME_COMPONENT_MAX_BYTES;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.to_mut().truncate(end);
    }
}

/// Build the randomized landing name for a mail-controlled hint.
///
/// CVE-2002-1210 (issue #59, Eudora 5.1.1/5.2): the client stored an
/// attachment at a predictable path, and a link inside the message
/// pulled that file back through `file://` so a browser executed it in
/// the local context. meli's equivalent surface is the default
/// temp-file landing — the mailcap `%s` expansion and the open-with
/// default application both materialize the mail-controlled hint under
/// `<temp_dir>/meli/`. The old spelling landed exactly at
/// `<temp_dir>/meli/<hint>`, with no random component, so the sender
/// could predict the path (and pre-plant a file at it). Insert an
/// unguessable UUID v4 infix before the extension so the landing path
/// is not predictable from the message, while keeping the sanitized
/// stem as a recognizable prefix.
///
/// `filename` is already sanitized and capped to
/// [`FILENAME_COMPONENT_MAX_BYTES`]. The returned component is capped
/// as well: the UUID infix and the preserved extension are budgeted out
/// of the stem on a UTF-8 character boundary. The extension is kept only
/// when it follows a non-empty stem (so a leading `.` is a hidden-file
/// name, not an extension) and is at most `32` bytes long.
fn randomized_temp_component(filename: &str) -> String {
    /// `Uuid::as_simple` renders exactly 32 lowercase hex digits.
    const UUID_INFIX_HEX_LEN: usize = 32;
    // Split at the last `.`; a missing/empty stem, an empty extension
    // and an over-long extension all mean "no extension to preserve",
    // in which case the whole hint stays the stem.
    let (stem, ext) = match filename.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 32 => {
            (stem, Some(ext))
        }
        _ => (filename, None),
    };
    // `_` + the 32 hex digits + (`.` + ext) must fit next to the stem
    // inside the component cap.
    let reserved = 1 + UUID_INFIX_HEX_LEN + ext.map_or(0, |ext| 1 + ext.len());
    let mut stem_end = FILENAME_COMPONENT_MAX_BYTES
        .saturating_sub(reserved)
        .min(stem.len());
    while !stem.is_char_boundary(stem_end) {
        stem_end -= 1;
    }
    let stem = &stem[..stem_end];
    let infix = Uuid::new_v4();
    match ext {
        Some(ext) => format!("{stem}_{}.{ext}", infix.as_simple()),
        None => format!("{stem}_{}", infix.as_simple()),
    }
}

pub fn sanitize_filename(value: &mut Cow<'_, str>) {
    // Replace with <https://docs.rs/regex/latest/regex/macro.regex.html> when we update the regex
    // dependency
    macro_rules! regex {
        ($re:literal) => {{
            static REGEX: std::sync::LazyLock<regex::Regex> =
                std::sync::LazyLock::new(|| regex::Regex::new($re).expect("invalid regex pattern"));

            // Coerce returned type from `&Lazy<Regex>` to `&Regex` to avoid making the
            // inner type public.
            let re: &regex::Regex = &REGEX;
            re
        }};
    }

    // Macro to detect whether <regex>.replace_all performed no replacements, because it returns a
    // Cow::Borrowed that borrowes the _haystack_ and not the function argument `value`'s lifetime.
    macro_rules! replace_all {
        ($re:expr, $with:literal) => {{
            let re = $re;
            match re.replace_all(value.as_ref(), $with) {
                Cow::Owned(owned) => {
                    *value = Cow::Owned(owned);
                }
                Cow::Borrowed(_haystack) => {}
            }
        }};
    }

    sanitize_separator(value);

    replace_all!(regex!(r"(?m)[[:space:]]+"), "_");
    replace_all!(regex!(r#"(?m)[!"'/\\]+"#), "-");
    replace_all!(regex!(r"(?m)[[:cntrl:]]*"), "");
    replace_all!(regex!(r"(?m)[[:blank:]]*"), "");
    replace_all!(regex!(r#"^[!"'/\\]*"#), "");
    replace_all!(regex!(r"(?m)__+"), "_");
    replace_all!(regex!(r#"[!"'/\\]*$"#), "");
}

/// Sanitize a mail-controlled filename into a single safe path
/// component, returning whether what remains is a usable file name.
///
/// This is [`sanitize_filename`] plus a guard against the special
/// relative names: `false` for `""`/`"."`/`".."` so the caller falls
/// back to a generated name (CVE-2024-43604 equivalent surface: an
/// attachment filename must never carry path components, separators
/// or control characters into a save path).
pub fn sanitize_filename_component(value: &mut Cow<'_, str>) -> bool {
    sanitize_filename(value);
    // A usable component is also a bounded one: over-length names are
    // truncated instead of failing the filesystem write later
    // (CVE-2003-0376 regression, see [`FILENAME_COMPONENT_MAX_BYTES`]).
    cap_filename_component_bytes(value);
    !matches!(value.as_ref(), "" | "." | "..")
}

/// Claim `filename` against `used`, the names already taken in the
/// same destination directory, returning the (possibly suffixed) name
/// that is now reserved.
///
/// The batch save paths sanitize a mail-controlled name *first* and
/// claim it here *second*: two different hostile spellings that
/// flatten to the same sanitized component (CVE-2024-43604 review:
/// `../../twin.bin` and `..\..\twin.bin`) must both land — the second
/// with a `_<n>` inserted before its extension — instead of failing
/// the `create_new` write and silently dropping one attachment. A name
/// without an extension takes the suffix at its end.
pub fn unique_filename_component(
    used: &mut std::collections::HashSet<String>,
    filename: &str,
) -> String {
    if used.insert(filename.to_string()) {
        return filename.to_string();
    }
    let (stem, dot_ext) = match filename.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_string(), format!(".{ext}")),
        None => (filename.to_string(), String::new()),
    };
    let mut dedup = 1;
    loop {
        let candidate = format!("{stem}_{dedup}{dot_ext}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        dedup += 1;
    }
}

/// Default `.eml` name for saving a message to disk, derived from its
/// `Message-ID`.
///
/// The identifier is mail-controlled, so it is sanitized into one safe
/// path component — a hostile identifier (`../../evil`,
/// `/etc/cron.d/pwn`, control characters) must not traverse out of the
/// destination directory. The stem is sanitized *before* the `.eml`
/// suffix is appended: an identifier that sanitizes down to nothing —
/// empty (a mail with no `Message-ID`), control-only, or the `.`/`..`
/// spellings — falls back to a generated name instead of leaving the
/// bare `.eml` hidden dotfile every such mail would collide on under
/// the `create_new` write (CVE-2024-43604 review fix).
///
/// The fallback embeds a random UUID v4 on purpose: unpredictable (an
/// attacker cannot pre-place a file at the landing name) and unique
/// (repeated exports never collide with each other), which a name
/// derived from the hostile identifier itself could not guarantee.
pub fn eml_filename(message_id: &str) -> String {
    let mut stem = Cow::Borrowed(message_id);
    if !sanitize_filename_component(&mut stem) {
        return format!("meli_mail_{}.eml", Uuid::new_v4().as_simple());
    }
    format!("{stem}.eml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_invalid_path() {
        let f = File {
            path: PathBuf::from("//////"),
            delete_on_drop: true,
        };
        f.as_std_file().unwrap_err();
    }

    #[test]
    fn test_file_delete_on_drop() {
        const S: &str = "hello world";
        let tempdir = tempfile::tempdir().unwrap();

        let delete_on_drop = File::create_temp_file(
            S.as_bytes(),
            None,
            Some(&mut tempdir.path().join("test")),
            None,
            true,
        )
        .unwrap();
        assert_eq!(&delete_on_drop.read_to_string().unwrap(), S);
        drop(delete_on_drop);
        assert!(!tempdir.path().join("test").try_exists().unwrap());

        let persist = File::create_temp_file(
            S.as_bytes(),
            None,
            Some(&mut tempdir.path().join("test")),
            None,
            false,
        )
        .unwrap();
        assert_eq!(&persist.read_to_string().unwrap(), S);
        drop(persist);
        assert!(tempdir.path().join("test").try_exists().unwrap());

        _ = tempdir.close();
    }

    #[test]
    fn test_file_sanitize_filename() {
        const OK_FILENAME: &str = "okay";
        const PATH_SEP_FILENAME: &str = "meli/meli/issues/712/comment/4492@git.meli-email.org";
        const EFFED_UP_FILENAME: &str = "Re: Some long subject - \"User Dot. Name\" \
                                         <user1@example.com> Sent from my bPad 2024-09-07, on   a \
                                         sunny Saturday";

        let mut filename = Cow::Borrowed(OK_FILENAME);
        sanitize_filename(&mut filename);
        assert_eq!(filename, Cow::<'static, str>::Borrowed(OK_FILENAME));
        sanitize_separator(&mut filename);
        assert_eq!(filename, Cow::<'static, str>::Borrowed(OK_FILENAME));

        let mut filename = Cow::Borrowed(PATH_SEP_FILENAME);
        sanitize_filename(&mut filename);
        assert_eq!(
            filename,
            Cow::<'static, str>::Owned(
                "meli_meli_issues_712_comment_4492@git.meli-email.org".to_string()
            )
        );
        let mut filename = Cow::Borrowed(PATH_SEP_FILENAME);
        sanitize_separator(&mut filename);
        assert_eq!(
            filename,
            Cow::<'static, str>::Owned(
                "meli_meli_issues_712_comment_4492@git.meli-email.org".to_string()
            )
        );

        let mut filename = Cow::Borrowed(EFFED_UP_FILENAME);
        sanitize_filename(&mut filename);
        assert_eq!(
            filename,
            Cow::<'static, str>::Owned(
                "Re:_Some_long_subject_-_-User_Dot._Name-_<user1@example.\
                 com>_Sent_from_my_bPad_2024-09-07,_on_a_sunny_Saturday"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_file_sanitize_filename_punctuation() {
        // Sanitization is narrowed to `!"'/\`; other punctuation such as `@`
        // and `.` must be preserved while the targeted punctuation is cleaned.
        let mut filename = Cow::Borrowed("!bang's@example.com");
        sanitize_filename(&mut filename);
        assert_eq!(
            filename,
            Cow::<'static, str>::Owned("-bang-s@example.com".to_string())
        );
    }

    /// CVE-2025-47176 regression (issue #26): the separator sanitizer must
    /// strip both `/` and `\` on every platform, so that no spelling of
    /// the path-traversal lexeme family survives into a `Path::push`:
    /// the literal `'.../...//'` of the advisory, plain and backslash
    /// `../..` variants, mixed spellings, absolute paths, UNC shares and
    /// drive letters.
    #[test]
    fn test_file_sanitize_separator_path_traversal() {
        for (raw, flat) in [
            (".../...//evil.exe", "..._...__evil.exe"),
            ("../..//evil.exe", ".._..__evil.exe"),
            (r"..\..\evil.exe", ".._.._evil.exe"),
            (r"..\/..//evil.exe", "..__..__evil.exe"),
            ("/absolute/evil.exe", "_absolute_evil.exe"),
            (r"\\attacker\share\evil.exe", "__attacker_share_evil.exe"),
            (r"C:\Temp\evil.exe", "C:_Temp_evil.exe"),
            ("plain.exe", "plain.exe"),
        ] {
            let mut value = Cow::Borrowed(raw);
            sanitize_separator(&mut value);
            assert_eq!(value, Cow::<'static, str>::Borrowed(flat), "raw {raw:?}");
            let mut full = Cow::Borrowed(raw);
            sanitize_filename(&mut full);
            assert!(
                !full.contains('/')
                    && !full.contains('\\')
                    && !full.contains('\0')
                    && std::path::Path::new(full.as_ref()).components().count() == 1,
                "sanitize_filename must leave a flat component, got {full:?} (raw {raw:?})"
            );
        }
    }

    /// Mail-controlled filenames must normalize into a single safe path
    /// component: path separators of both kinds and control characters
    /// are stripped, and the special relative names are refused
    /// (CVE-2024-43604 regression).
    #[test]
    fn test_sanitize_filename_component_strips_path_and_control_characters() {
        for raw in [
            "../../../.config/evil.conf",
            "/etc/cron.d/pwn",
            "..\\..\\evil.exe",
            "report\u{1b}[2j.pdf",
            "evil\r\nname",
        ] {
            let mut value = Cow::Borrowed(raw);
            assert!(
                sanitize_filename_component(&mut value),
                "{raw:?} must remain a usable name"
            );
            let out = value.as_ref();
            assert!(!out.contains('/'), "{raw:?} -> {out:?}");
            assert!(!out.contains('\\'), "{raw:?} -> {out:?}");
            assert!(
                out.chars().all(|c| !c.is_control()),
                "{raw:?} -> {out:?} keeps control characters"
            );
        }
        for raw in ["", ".", ".."] {
            let mut value = Cow::Borrowed(raw);
            assert!(
                !sanitize_filename_component(&mut value),
                "{raw:?} must be refused as a file name"
            );
        }
    }

    /// CVE-2003-0376 regression (issue #47, Eudora 5.2.1 "Attachment
    /// Converted" dot-pile overflow): the sanitized name hint may be
    /// arbitrarily long, but temp-file materialization must stay
    /// bounded and flat. Before the byte cap, a name past `NAME_MAX`
    /// fell into the per-grapheme `ENAMETOOLONG` retry loop —
    /// O(len²) work, minutes of frozen client on one hostile mail.
    #[test]
    fn test_create_temp_file_caps_overlong_name_hint() {
        for len in [260usize, 10_000, 100_000] {
            let name: String = "a".repeat(len - 4) + ".exe";
            let start = std::time::Instant::now();
            let file = File::create_temp_file(b"corpus", Some(&name), None, None, false)
                .unwrap_or_else(|err| panic!("len {len}: {err}"));
            let elapsed = start.elapsed();
            let path = file.path().to_path_buf();
            let component = path.file_name().unwrap().to_str().unwrap().to_string();
            assert!(
                path.starts_with(std::env::temp_dir().join("meli")),
                "len {len}: landed outside the temp root: {}",
                path.display()
            );
            assert!(
                component.len() <= FILENAME_COMPONENT_MAX_BYTES,
                "len {len}: component must be capped, got {} bytes: {component:?}",
                component.len()
            );
            assert!(component.starts_with('a'), "len {len}: {component:?}");
            assert!(!component.contains('/'), "len {len}: {component:?}");
            assert!(!component.contains('\\'), "len {len}: {component:?}");
            assert_eq!(std::fs::read(&path).unwrap(), b"corpus", "len {len}");
            assert!(
                elapsed < std::time::Duration::from_secs(5),
                "len {len}: materialization took {elapsed:?} — the retry loop is unbounded again"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// CVE-2003-0376 regression (issue #47): a name that is one huge
    /// pile of dots — the literal Eudora trigger shape — sanitizes
    /// into one flat, bounded, usable component; the `.`/`..`
    /// degenerate spellings keep falling back to the generated name.
    #[test]
    fn test_create_temp_file_survives_dot_pile_hints() {
        for dots in [22usize, 122, 1000, 100_000] {
            let name = format!("a{}.exe", ".".repeat(dots));
            let file = File::create_temp_file(b"corpus", Some(&name), None, None, false)
                .unwrap_or_else(|err| panic!("dots {dots}: {err}"));
            let path = file.path().to_path_buf();
            let component = path.file_name().unwrap().to_str().unwrap().to_string();
            assert!(
                path.starts_with(std::env::temp_dir().join("meli")),
                "dots {dots}: landed outside the temp root: {}",
                path.display()
            );
            assert!(
                !component.contains('/') && !component.contains('\\'),
                "dots {dots}: {component:?}"
            );
            assert!(
                component.chars().all(|c| !c.is_control()),
                "dots {dots}: {component:?}"
            );
            assert!(
                !matches!(component.as_str(), "" | "." | ".."),
                "dots {dots}: {component:?} is not a real file name"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"corpus", "dots {dots}");
            let _ = std::fs::remove_file(&path);
        }
        // The degenerate hints still fall back to a generated name.
        for raw in ["", ".", ".."] {
            let file = File::create_temp_file(b"corpus", Some(raw), None, None, false)
                .unwrap_or_else(|err| panic!("{raw:?}: {err}"));
            let path = file.path().to_path_buf();
            let component = path.file_name().unwrap().to_str().unwrap().to_string();
            assert!(
                component.len() <= FILENAME_COMPONENT_MAX_BYTES,
                "{raw:?}: {component:?}"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// CVE-2003-0376 regression (issue #47): the component normalizer
    /// caps over-length names on a character boundary and keeps them
    /// usable, while the dot-pile shapes stay flat single components.
    #[test]
    fn test_sanitize_filename_component_caps_overlength() {
        for len in [260usize, 10_000, 100_000] {
            let name: String = "a".repeat(len - 4) + ".exe";
            let mut value = Cow::Borrowed(name.as_str());
            assert!(sanitize_filename_component(&mut value), "len {len}");
            let out = value.as_ref();
            assert!(
                out.len() <= FILENAME_COMPONENT_MAX_BYTES,
                "len {len}: {out:?} ({} bytes)",
                out.len()
            );
            assert!(out.starts_with('a'), "len {len}: {out:?}");
            assert!(
                std::path::Path::new(out).components().count() == 1,
                "len {len}: {out:?} is not one component"
            );
        }
        // Dot piles — the literal Eudora trigger — stay flat and
        // usable, never special relative components.
        for dots in [22usize, 122, 1000, 100_000] {
            let name = format!("a{}.exe", ".".repeat(dots));
            let mut value = Cow::Borrowed(name.as_str());
            assert!(sanitize_filename_component(&mut value), "dots {dots}");
            let out = value.as_ref();
            assert!(!out.contains('/'), "dots {dots}: {out:?}");
            assert!(out.chars().all(|c| !c.is_control()), "dots {dots}: {out:?}");
        }
    }

    /// The `.eml` export name derived from a hostile `Message-ID` must
    /// be one flat component — no traversal, no absolute replacement,
    /// no control characters — with a generated fallback for the
    /// degenerate identifiers (CVE-2024-43604 regression).
    #[test]
    fn test_eml_filename_neutralizes_hostile_message_ids() {
        for hostile in [
            "../../evil",
            "/etc/cron.d/pwn",
            "..\\..\\evil",
            "evil\u{1b}[2j",
            "!",
        ] {
            let name = eml_filename(hostile);
            assert!(name.ends_with(".eml"), "{hostile:?} -> {name:?}");
            assert!(!name.contains('/'), "{hostile:?} -> {name:?}");
            assert!(!name.contains('\\'), "{hostile:?} -> {name:?}");
            assert!(
                name.chars().all(|c| !c.is_control()),
                "{hostile:?} -> {name:?} keeps control characters"
            );
        }
        // Identifiers with no usable stem after sanitization — including
        // the empty one a mail with no `Message-ID` carries — take the
        // generated fallback instead of a bare/hidden `.eml` name
        // (CVE-2024-43604 review fix).
        for degenerate in ["..", ".", "", "\u{1b}\u{7f}"] {
            let name = eml_filename(degenerate);
            assert!(
                name.starts_with("meli_mail_") && name.ends_with(".eml"),
                "{degenerate:?} -> {name:?} must take the generated fallback"
            );
            assert!(
                !name.starts_with('.'),
                "{degenerate:?} -> {name:?} must not be a hidden dotfile"
            );
            assert!(
                name.chars().all(|c| !c.is_control()),
                "{degenerate:?} -> {name:?} keeps control characters"
            );
        }
        // Normal identifiers keep their name byte-for-byte.
        assert_eq!(eml_filename("abc@def.example"), "abc@def.example.eml");
    }

    /// Batch-name claiming runs on the *sanitized* name: hostile
    /// spellings that flatten to the same component dedup with a
    /// `_<n>` suffix before the extension instead of colliding at the
    /// filesystem (CVE-2024-43604 review interaction).
    #[test]
    fn test_unique_filename_component_dedups_after_sanitization() {
        fn sanitize_and_claim(used: &mut std::collections::HashSet<String>, raw: &str) -> String {
            let mut name = Cow::Borrowed(raw);
            assert!(sanitize_filename_component(&mut name), "{raw:?} is usable");
            unique_filename_component(used, name.as_ref())
        }

        let mut used = std::collections::HashSet::new();
        assert_eq!(
            sanitize_and_claim(&mut used, "../../twin.bin"),
            ".._.._twin.bin"
        );
        assert_eq!(
            sanitize_and_claim(&mut used, "..\\..\\twin.bin"),
            ".._.._twin_1.bin"
        );
        assert_eq!(
            sanitize_and_claim(&mut used, "../../twin.bin"),
            ".._.._twin_2.bin"
        );
        assert_eq!(sanitize_and_claim(&mut used, "report.pdf"), "report.pdf");

        // Extension-less names take the suffix at the end.
        let mut used = std::collections::HashSet::new();
        assert_eq!(unique_filename_component(&mut used, "README"), "README");
        assert_eq!(unique_filename_component(&mut used, "README"), "README_1");
    }

    /// A mail-controlled name hint that sanitizes down to `""`/`.`/`..`
    /// must not be pushed into the target directory: the temp file
    /// falls back to a generated name inside it
    /// (CVE-2024-43604 regression, the mailcap `%s` landing path).
    #[test]
    fn test_file_temp_filename_degenerate_name_falls_back() {
        let tempdir = tempfile::tempdir().unwrap();
        let target = tempdir.path().join("meli-cve-43604");
        std::fs::create_dir_all(&target).unwrap();
        for degenerate in ["", ".", ".."] {
            let mut target = target.clone();
            let file =
                File::create_temp_file(b"x", Some(degenerate), Some(&mut target), None, true)
                    .unwrap();
            let path = file.path();
            assert!(
                path.starts_with(target.parent().unwrap()),
                "{degenerate:?} -> {path:?} must stay inside the target directory"
            );
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(
                !matches!(name, "" | "." | ".."),
                "{degenerate:?} -> {name:?} must be a real file name"
            );
            assert_eq!(std::fs::read(path).unwrap(), b"x");
        }
        _ = tempdir.close();
    }

    /// CVE-2002-1210 regression (issue #59): the default temp-file
    /// landing must not be predictable from the mail-controlled hint.
    /// The old spelling landed exactly at `<temp_dir>/meli/<hint>`, the
    /// path a `file://` link in the message could point a browser at
    /// (Eudora 5.1.1/5.2). Every default-branch landing now carries a
    /// random UUID v4 infix before its extension: two calls on the same
    /// hint land on different paths, while the sanitized stem and the
    /// extension stay recognizable and the component stays bounded.
    #[test]
    fn test_create_temp_file_default_landing_is_randomized() {
        const HINT: &str = "eudora_leak.htm";
        let temp_root = std::env::temp_dir().join("meli");

        let first = File::create_temp_file(b"one", Some(HINT), None, None, false).unwrap();
        let second = File::create_temp_file(b"two", Some(HINT), None, None, false).unwrap();
        assert_ne!(
            first.path(),
            second.path(),
            "the same hint must not land on the same path twice"
        );

        for file in [&first, &second] {
            let path = file.path();
            assert!(
                path.starts_with(&temp_root),
                "must land under <temp_dir>/meli, got {}",
                path.display()
            );
            let name = path.file_name().unwrap().to_str().unwrap();
            assert_ne!(name, HINT, "the raw hint must never be the landing name");
            assert!(
                name.starts_with("eudora_leak_"),
                "the sanitized stem must stay a recognizable prefix: {name:?}"
            );
            assert!(
                name.ends_with(".htm"),
                "the extension must be kept: {name:?}"
            );
            assert!(
                name.len() <= FILENAME_COMPONENT_MAX_BYTES,
                "landing component must stay capped, got {} bytes: {name:?}",
                name.len()
            );
            // The infix is the last `_`-separated field before the
            // extension: exactly 32 lowercase hex digits.
            let infix = name
                .strip_suffix(".htm")
                .unwrap()
                .rsplit_once('_')
                .expect("the UUID infix must be present")
                .1;
            assert_eq!(infix.len(), 32, "UUID infix width: {name:?}");
            assert!(
                infix
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "UUID infix must be lowercase hex: {name:?}"
            );
        }
        assert_eq!(std::fs::read(first.path()).unwrap(), b"one");
        assert_eq!(std::fs::read(second.path()).unwrap(), b"two");
        let _ = std::fs::remove_file(first.path());
        let _ = std::fs::remove_file(second.path());
    }

    /// CVE-2002-1210 regression (issue #59): a file pre-planted at the
    /// old predictable landing path is neither used nor clobbered — the
    /// randomized `create_new` landing goes elsewhere and the planted
    /// bytes survive untouched.
    #[test]
    fn test_create_temp_file_ignores_planted_predictable_path() {
        const HINT: &str = "cve_2002_1210_planted.htm";
        let planted = std::env::temp_dir().join("meli").join(HINT);
        std::fs::create_dir_all(planted.parent().unwrap()).unwrap();
        std::fs::write(&planted, b"attacker").unwrap();

        let file = File::create_temp_file(b"victim", Some(HINT), None, None, false).unwrap();
        assert_ne!(file.path(), planted.as_path());
        assert_eq!(std::fs::read(&planted).unwrap(), b"attacker");
        assert_eq!(std::fs::read(file.path()).unwrap(), b"victim");
        let _ = std::fs::remove_file(&planted);
        let _ = std::fs::remove_file(file.path());
    }

    /// CVE-2002-1210 regression (issue #59): `<temp_dir>/meli` must not
    /// be world-listable. The default umask left it 0o755, exposing the
    /// attachment names a victim lands; the staging directory is created
    /// (and, when it already exists, best-effort re-tightened)
    /// owner-only.
    #[cfg(unix)]
    #[test]
    fn test_temp_staging_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let file =
            File::create_temp_file(b"corpus", Some("perms_probe.htm"), None, None, false).unwrap();
        let mode = std::fs::metadata(std::env::temp_dir().join("meli"))
            .expect("the meli staging directory must exist after a landing")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "the staging directory must be owner-only, got {mode:o}"
        );
        let _ = std::fs::remove_file(file.path());
    }

    /// Landed temp files stay owner-only (`0o600`): the randomized name
    /// changes the path, not the `create_new` private-mode write.
    #[cfg(unix)]
    #[test]
    fn test_create_temp_file_default_landing_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let file =
            File::create_temp_file(b"corpus", Some("secret.htm"), None, None, false).unwrap();
        let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "landed temp files must be readable by the owner only"
        );
        let _ = std::fs::remove_file(file.path());
    }
}
