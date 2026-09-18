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

//! # mailcap file - Find mailcap entries to execute attachments.
//!
//! Implements [RFC1524 A User Agent Configuration Mechanism For Multimedia
//! Mail Format Information](https://www.rfc-editor.org/rfc/inline-errata/rfc1524.html)

use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

use melib::{email::Attachment, log, utils::fnmatch::Fnmatch, Error, Result};

use crate::{
    components::ComponentId,
    state::Context,
    types::{File, NotificationType, ProcessResultFn, SpawnInteractionFn, UIEvent},
};

macro_rules! split_command {
    ($cmd:expr) => {{
        $cmd.split_whitespace().collect::<Vec<&str>>()
    }};
}

/// Quote a value for safe interpolation into a shell command line using
/// POSIX single-quote semantics: the value is wrapped in `'...'` and any
/// embedded `'` becomes `'\''`.
fn quote_shell_word(s: &str) -> String {
    let mut ret = String::with_capacity(s.len() + 2);
    ret.push('\'');
    for c in s.chars() {
        if c == '\'' {
            ret.push_str("'\\''");
        } else {
            ret.push(c);
        }
    }
    ret.push('\'');
    ret
}

/// Find the first matching mailcap entry in `content` for `content_type`.
///
/// Returns the raw command field and whether `copiousoutput` was set. Lines
/// that are malformed (no `;` separator, no command field) are skipped: a
/// mailcap file is external input and must never make meli panic.
fn lookup_mailcap_entry(content: &str, content_type: &str) -> Option<MailcapEntry> {
    let mut lines_iter = content.lines();
    while let Some(l) = lines_iter.next() {
        let l = l.trim();
        if l.starts_with('#') {
            continue;
        }
        if l.is_empty() {
            continue;
        }

        let l = if let Some(stripped) = l.strip_suffix('\\') {
            // Continuation line: the backslash is removed and the next line
            // is appended. A trailing backslash on the last line has no
            // continuation; never slice by byte arithmetic because the line
            // can be shorter than two bytes or end in a multi-byte character.
            match lines_iter.next() {
                Some(next) => format!("{stripped}{next}"),
                None => stripped.to_string(),
            }
        } else {
            l.to_string()
        };
        let mut parts_iter = l.split(';');
        let Some(key) = parts_iter.next() else {
            continue;
        };
        let Some(cmd) = parts_iter.next() else {
            // A mailcap line without a command field (no `;`) is malformed.
            log::trace!("malformed mailcap line (no command field): {l}");
            continue;
        };
        if key.starts_with(content_type) || content_type.fnmatches(key) {
            let mut copiousoutput = false;
            #[allow(clippy::while_let_on_iterator)]
            while let Some(flag) = parts_iter.next() {
                if flag.trim() == "copiousoutput" {
                    copiousoutput = true;
                } else {
                    log::trace!("unknown mailcap flag: {}", flag);
                }
            }

            return Some(MailcapEntry {
                command: cmd.to_string(),
                copiousoutput,
            });
        }
    }
    None
}

pub struct MailcapEntry {
    command: String,
    /* Pass to pager */
    copiousoutput: bool,
}

impl MailcapEntry {
    pub fn execute(owner: ComponentId, a: &Attachment, context: &mut Context) -> Result<()> {
        /* lookup order:
         *  $XDG_CONFIG_HOME/meli/mailcap:$XDG_CONFIG_HOME/.mailcap:$HOME/.mailcap:/
         * etc/mailcap:/usr/etc/mailcap:/usr/local/etc/mailcap
         */
        let xdg_dirs =
            xdg::BaseDirectories::with_prefix("meli").map_err(|e| Error::new(e.to_string()))?;
        let mut mailcap_path = xdg_dirs
            .place_config_file("mailcap")
            .map_err(|e| Error::new(e.to_string()))?;
        if !mailcap_path.exists() {
            mailcap_path = xdg::BaseDirectories::new()
                .map_err(|e| Error::new(e.to_string()))?
                .place_config_file("mailcap")?;
            if !mailcap_path.exists() {
                if let Ok(home) = std::env::var("HOME") {
                    mailcap_path = PathBuf::from(format!("{home}/.mailcap"));
                }
                if !mailcap_path.exists() {
                    mailcap_path = PathBuf::from("/etc/mailcap");
                    if !mailcap_path.exists() {
                        mailcap_path = PathBuf::from("/usr/etc/mailcap");
                        if !mailcap_path.exists() {
                            mailcap_path = PathBuf::from("/usr/local/etc/mailcap");
                        }
                        if !mailcap_path.exists() {
                            return Err(Error::new("No mailcap file found."));
                        }
                    }
                }
            }
        }

        let mut content = String::new();

        std::fs::File::open(mailcap_path.as_path())?.read_to_string(&mut content)?;
        let content_type = a.content_type().to_string();

        match lookup_mailcap_entry(&content, &content_type) {
            None => Err(Error::new("Not found")),
            Some(Self {
                command,
                copiousoutput,
            }) => {
                let parts = split_command!(command);
                let Some((cmd, args)) = parts.split_first() else {
                    return Err(Error::new(format!(
                        "Malformed mailcap entry for `{content_type}`: command is empty"
                    )));
                };
                let mut needs_stdin = true;
                let params = a.parameters();
                /* [ref:TODO]: See mailcap(5)
                 * - replace "\%" with "%" and unescape other blackslash uses.
                 * - "%n" and "%F".
                 * - test=xxx field.
                 */
                let args = args
                    .iter()
                    .map(|arg| match *arg {
                        "%s" => {
                            needs_stdin = false;
                            let file = File::create_temp_file(
                                &a.decode(Default::default()),
                                None,
                                None,
                                None,
                                false,
                            )?;
                            let p = file.path().display().to_string();
                            Ok(quote_shell_word(&p))
                        }
                        "%t" => Ok(quote_shell_word(&a.content_type().to_string())),
                        param if param.starts_with("%{") && param.ends_with('}') => {
                            let param = &param["%{".len()..param.len() - 1];
                            let value = if let Some(v) =
                                params.iter().find(|(k, _)| *k == param.as_bytes())
                            {
                                String::from_utf8_lossy(v.1).into()
                            } else if param == "charset" {
                                String::from("utf-8")
                            } else {
                                String::new()
                            };
                            Ok(quote_shell_word(&value))
                        }
                        a => Ok(a.to_string()),
                    })
                    .collect::<Result<Vec<String>>>()?;
                let decoded_bytes = if needs_stdin {
                    Some(a.decode(Default::default()))
                } else {
                    None
                };
                let cmd_string = format!("{} {}", cmd, args.join(" "));
                if copiousoutput {
                    context.replies.push_back(UIEvent::ProcessRequest {
                        owner,
                        command: {
                            let mut cmd = Command::new("sh");
                            cmd.args(["-c", &cmd_string])
                                .stdin(Stdio::piped())
                                .stdout(Stdio::piped())
                                .stderr(Stdio::piped());
                            cmd
                        },
                        spawn: Some(SpawnInteractionFn(Box::new(move |mut child| {
                            if let Some(decoded_bytes) = decoded_bytes {
                                child
                                    .stdin
                                    .as_mut()
                                    .expect("handle present")
                                    .write_all(&decoded_bytes)?;
                            }
                            Ok(child)
                        }))),
                        result_cb: ProcessResultFn(Box::new(move |output| {
                            let output = match output {
                                Ok(v) => v,
                                Err(err) => {
                                    return Some(Box::new(UIEvent::Notification {
                                        title: None,
                                        source: None,
                                        body: err.to_string().into(),
                                        kind: Some(NotificationType::Error(err.kind)),
                                    }));
                                }
                            };
                            let pager_cmd = if let Ok(v) = std::env::var("PAGER") {
                                std::borrow::Cow::from(v)
                            } else {
                                std::borrow::Cow::from("less")
                            };

                            Some(Box::new(UIEvent::ProcessRequest {
                                owner,
                                command: {
                                    let mut cmd = Command::new("sh");
                                    cmd.args(["-c", &pager_cmd])
                                        .stdin(Stdio::piped())
                                        .stdout(Stdio::inherit())
                                        .stderr(Stdio::inherit());
                                    cmd
                                },
                                spawn: Some(SpawnInteractionFn(Box::new(move |mut child| {
                                    child
                                        .stdin
                                        .as_mut()
                                        .expect("handle present")
                                        .write_all(&output.stdout)?;
                                    Ok(child)
                                }))),
                                result_cb: ProcessResultFn(Box::new(|_output| {
                                    log::trace!("output = {_output:?}");
                                    None
                                })),
                            }))
                        })),
                    });
                } else if let Some(decoded_bytes) = decoded_bytes {
                    context.replies.push_back(UIEvent::ProcessRequest {
                        owner,
                        command: {
                            let mut cmd = Command::new("sh");
                            cmd.args(["-c", &cmd_string])
                                .stdin(Stdio::piped())
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit());
                            cmd
                        },
                        spawn: Some(SpawnInteractionFn(Box::new(move |mut child| {
                            child
                                .stdin
                                .as_mut()
                                .expect("handle present")
                                .write_all(&decoded_bytes)?;
                            Ok(child)
                        }))),
                        result_cb: ProcessResultFn(Box::new(|_output| {
                            log::trace!("output = {_output:?}");
                            None
                        })),
                    });
                } else {
                    context.replies.push_back(UIEvent::ProcessRequest {
                        owner,
                        command: {
                            let mut cmd = Command::new("sh");
                            cmd.args(["-c", &cmd_string])
                                .stdin(Stdio::inherit())
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit());
                            cmd
                        },
                        spawn: Some(SpawnInteractionFn::default()),
                        result_cb: ProcessResultFn(Box::new(|_output| {
                            log::trace!("output = {_output:?}");
                            None
                        })),
                    });
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::lookup_mailcap_entry;

    /// Flatten the lookup result to the tuple the tests compare against.
    fn entry(content: &str, content_type: &str) -> Option<(String, bool)> {
        lookup_mailcap_entry(content, content_type).map(|e| (e.command, e.copiousoutput))
    }

    /// Malformed lines (no command field, truncated or non-ASCII
    /// continuation) must be skipped instead of panicking.
    #[test]
    fn malformed_mailcap_lines_are_skipped() {
        for content in [
            "text/plain", // no `;`
            "\\",         // lone backslash
            "é\\",        // continuation after a multi-byte char
            "",           // empty file
        ] {
            assert!(
                entry(content, "application/pdf").is_none(),
                "{content:?} must yield no entry"
            );
        }

        // A trailing continuation backslash on the last line has no next line:
        // it is treated as a literal (removed) instead of panicking.
        assert_eq!(
            entry("application/pdf;cmd\\", "application/pdf"),
            Some(("cmd".to_string(), false))
        );

        // A malformed line before a valid one must not abort the scan.
        let content = "text/plain\napplication/pdf;pdfviewer %s;copiousoutput\n";
        assert_eq!(
            entry(content, "application/pdf"),
            Some(("pdfviewer %s".to_string(), true))
        );
    }

    /// Valid entries keep matching exactly as before the hardening.
    #[test]
    fn valid_mailcap_entries_still_match() {
        let content = "image/*;feh %s\napplication/pdf;zathura %s\n";
        assert_eq!(
            entry(content, "image/png"),
            Some(("feh %s".to_string(), false))
        );
        assert_eq!(
            entry(content, "application/pdf"),
            Some(("zathura %s".to_string(), false))
        );
        assert!(entry(content, "text/plain").is_none());
    }

    /// A continuation line joins with the following line.
    #[test]
    fn mailcap_continuation_joins_lines() {
        let content = "application/pdf;pdfviewer \\\n%s\n";
        assert_eq!(
            entry(content, "application/pdf"),
            Some(("pdfviewer %s".to_string(), false))
        );
    }
}
