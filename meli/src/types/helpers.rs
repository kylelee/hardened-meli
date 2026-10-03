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
                std::fs::DirBuilder::new().recursive(true).create(&dir)?;
                if let Some(filename) = filename {
                    dir.push(filename);
                    'exists: while dir.try_exists().unwrap_or_default() {
                        for i in 0..u8::MAX {
                            dir.pop();
                            dir.push(format!("{filename}_{i}"));
                            if dir.try_exists().unwrap_or_default() {
                                break 'exists;
                            }
                        }
                        while dir.try_exists().unwrap_or_default() {
                            dir.pop();
                            dir.push(format!("{filename}_{}", Uuid::new_v4().as_simple()));
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

/// Remove system path separator from filename
#[inline(always)]
pub fn sanitize_separator(value: &mut Cow<'_, str>) {
    if value.contains(std::path::MAIN_SEPARATOR) {
        *value = Cow::Owned(value.replace(std::path::MAIN_SEPARATOR, "_"))
    };
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
    !matches!(value.as_ref(), "" | "." | "..")
}

/// Default `.eml` name for saving a message to disk, derived from its
/// `Message-ID`.
///
/// The identifier is mail-controlled, so it is sanitized into one safe
/// path component — a hostile identifier (`../../evil`,
/// `/etc/cron.d/pwn`, control characters) must not traverse out of the
/// destination directory. Falls back to a generated name when nothing
/// usable remains.
pub fn eml_filename(message_id: &str) -> String {
    let mut filename = Cow::Owned(format!("{message_id}.eml"));
    if !sanitize_filename_component(&mut filename) {
        return format!("meli_mail_{}.eml", Uuid::new_v4().as_simple());
    }
    filename.into_owned()
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
            "..",
            ".",
            "",
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
        // Normal identifiers keep their name byte-for-byte.
        assert_eq!(eml_filename("abc@def.example"), "abc@def.example.eml");
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
}
