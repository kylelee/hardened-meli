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

use std::{borrow::Cow, fs::File, io::Write, os::unix::fs::PermissionsExt, path::Path};

use melib::{Result, ShellExpandTrait};

pub fn save_attachment(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path.expand())?;
    let mut permissions = f.metadata()?.permissions();
    permissions.set_mode(0o600); // Read/write for owner only.
    f.set_permissions(permissions)?;
    f.write_all(bytes)?;
    f.flush()?;
    Ok(())
}

/// Middle-truncate `input` under the given char and byte caps.
///
/// When the input already fits it is returned as-is; otherwise the head
/// and the tail are kept and joined with a literal `"..."`. Char and byte
/// caps are honored per character along UTF-8 boundaries, so the result
/// is always valid UTF-8.
pub fn truncate_middle(input: &str, max_chars: usize, max_bytes: usize) -> Cow<'_, str> {
    if input.chars().count() <= max_chars && input.len() <= max_bytes {
        return Cow::Borrowed(input);
    }
    // The ASCII ellipsis costs 3 chars and 3 bytes on both budgets.
    let chars_budget = max_chars.saturating_sub(3);
    let bytes_budget = max_bytes.saturating_sub(3);
    if chars_budget == 0 || bytes_budget == 0 {
        // Degenerate budgets leave no room for an ellipsis: degrade to
        // plain head truncation under both caps (misuse guard; the
        // production call sites never pass such budgets).
        let mut end = 0;
        let mut bytes = 0;
        for (chars, (i, c)) in input.char_indices().enumerate() {
            let c_len = c.len_utf8();
            if chars >= max_chars || bytes + c_len > max_bytes {
                break;
            }
            bytes += c_len;
            end = i + c_len;
        }
        return Cow::Borrowed(&input[..end]);
    }
    let head_chars = chars_budget / 2;
    let tail_chars = chars_budget - head_chars;
    let head_bytes = bytes_budget / 2;
    let tail_bytes = bytes_budget - head_bytes;
    let mut head_end = 0;
    let mut bytes = 0;
    for (chars, (i, c)) in input.char_indices().enumerate() {
        let c_len = c.len_utf8();
        if chars >= head_chars || bytes + c_len > head_bytes {
            break;
        }
        bytes += c_len;
        head_end = i + c_len;
    }
    let mut tail_start = input.len();
    let mut bytes = 0;
    for (chars, (i, c)) in input.char_indices().rev().enumerate() {
        let c_len = c.len_utf8();
        if chars >= tail_chars || bytes + c_len > tail_bytes {
            break;
        }
        bytes += c_len;
        tail_start = i;
    }
    let head = &input[..head_end];
    let tail = &input[tail_start..];
    Cow::Owned(format!("{head}...{tail}"))
}

/// Parse the `Exec` value of a freedesktop.org desktop entry into arguments,
/// following the quoting rules of the Desktop Entry Specification, §7 "The
/// Exec key": arguments are separated by unquoted whitespace; double quotes
/// preserve whitespace; inside quotes only `"`, `` ` ``, `$` and `\` are
/// escapable by a backslash, and a backslash before any other character is
/// kept literally; single quotes are not special. Outside quotes a backslash
/// escapes the next character. An unterminated quote gracefully extends the
/// argument to the end of the string. (The `Exec` value is a single line by
/// spec, so newlines are treated as plain whitespace.)
fn parse_desktop_exec_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_arg = false;
    let mut in_quotes = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' => in_quotes = false,
                '\\' => match chars.next() {
                    Some(escaped @ ('"' | '`' | '$' | '\\')) => current.push(escaped),
                    Some(other) => {
                        current.push('\\');
                        current.push(other);
                    }
                    None => current.push('\\'),
                },
                other => current.push(other),
            }
        } else {
            match c {
                '"' => {
                    in_quotes = true;
                    has_arg = true;
                }
                '\\' => {
                    has_arg = true;
                    match chars.next() {
                        Some(escaped) => current.push(escaped),
                        None => current.push('\\'),
                    }
                }
                c if c.is_whitespace() => {
                    if has_arg {
                        args.push(std::mem::take(&mut current));
                        has_arg = false;
                    }
                }
                c => {
                    current.push(c);
                    has_arg = true;
                }
            }
        }
    }
    if has_arg {
        args.push(current);
    }
    args
}

/// Escape `arg` so that a shell running the result with `sh -c` passes it on
/// as a single, literal argument; a space becomes `\ `. Control characters
/// cannot survive shell quoting: a backslash before a newline is a line
/// continuation, so the shell drops both characters.
fn escape_sh_arg(arg: &str) -> Cow<'_, str> {
    fn is_safe(c: char) -> bool {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '.' | '/' | '-' | ':' | '@' | '%' | '+' | '=' | ',')
            || !c.is_ascii()
    }

    if arg.chars().all(is_safe) {
        return Cow::Borrowed(arg);
    }
    let mut escaped = String::with_capacity(arg.len() + 8);
    for c in arg.chars() {
        if !is_safe(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    Cow::Owned(escaped)
}

/// Replace the first occurrence of `code` with `replacement`; the
/// substituted value is not rescanned for further field codes.
fn substitute_field_code(args: &mut [String], code: &str, replacement: &str) -> bool {
    for arg in args.iter_mut() {
        if let Some(pos) = arg.find(code) {
            arg.replace_range(pos..pos + code.len(), replacement);
            return true;
        }
    }
    false
}

/// Expand the field codes of a desktop entry `Exec` value for a single
/// `path` (a URL when `is_url` is `true`) into a command line that a shell
/// invoked with `sh -c` executes with every argument intact.
pub fn desktop_exec_to_command(command: &str, path: String, is_url: bool) -> String {
    let url = if is_url {
        path.clone()
    } else {
        format!("file://{path}")
    };
    let mut args: Vec<String> = parse_desktop_exec_args(command)
        .into_iter()
        .map(|arg| {
            // Purge unused field codes; arguments that expand to nothing are
            // dropped entirely.
            arg.replace("%i", "").replace("%c", "").replace("%k", "")
        })
        .filter(|arg| !arg.is_empty())
        .collect();
    if !substitute_field_code(&mut args, "%f", &path)
        && !substitute_field_code(&mut args, "%F", &path)
        && !substitute_field_code(&mut args, "%u", &url)
        && !substitute_field_code(&mut args, "%U", &url)
    {
        args.push(path);
    }
    args.iter()
        .map(|arg| escape_sh_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desktop_exec() {
        assert_eq!(
            "ristretto /tmp/file".to_string(),
            desktop_exec_to_command("ristretto %F", "/tmp/file".to_string(), false)
        );
        assert_eq!(
            "/usr/lib/firefox-esr/firefox-esr file:///tmp/file".to_string(),
            desktop_exec_to_command(
                "/usr/lib/firefox-esr/firefox-esr %u",
                "/tmp/file".to_string(),
                false
            )
        );
        assert_eq!(
            "/usr/lib/firefox-esr/firefox-esr www.example.com".to_string(),
            desktop_exec_to_command(
                "/usr/lib/firefox-esr/firefox-esr %u",
                "www.example.com".to_string(),
                true
            )
        );
        assert_eq!(
            "/usr/bin/vlc --started-from-file www.example.com".to_string(),
            desktop_exec_to_command(
                "/usr/bin/vlc --started-from-file %U",
                "www.example.com".to_string(),
                true
            )
        );
        assert_eq!(
            "zathura --fork file:///tmp/file".to_string(),
            desktop_exec_to_command("zathura --fork %U", "file:///tmp/file".to_string(), true)
        );
    }

    #[test]
    fn test_desktop_exec_unquoted_freeze() {
        // Unquoted Exec values must keep producing the exact same command
        // string as the previous whitespace-splitting implementation.
        assert_eq!(
            "papers file:///tmp/doc.pdf".to_string(),
            desktop_exec_to_command("papers %U", "/tmp/doc.pdf".to_string(), false)
        );
        assert_eq!(
            "firefox https://example.org".to_string(),
            desktop_exec_to_command("firefox %u", "https://example.org".to_string(), true)
        );
        assert_eq!(
            "w3m -T text/html /tmp/a.html".to_string(),
            desktop_exec_to_command("w3m -T text/html", "/tmp/a.html".to_string(), false)
        );
        assert_eq!(
            "evince /tmp/x.pdf".to_string(),
            desktop_exec_to_command("evince %f", "/tmp/x.pdf".to_string(), false)
        );
        assert_eq!(
            "gimp /tmp/img\\ with\\ space.png".to_string(),
            desktop_exec_to_command("gimp %f", "/tmp/img with space.png".to_string(), false)
        );
    }

    #[test]
    fn test_desktop_exec_quoted_arguments() {
        assert_eq!(
            "/home/user/My\\ Docs/viewer /tmp/file".to_string(),
            desktop_exec_to_command(
                "\"/home/user/My Docs/viewer\" %f",
                "/tmp/file".to_string(),
                false
            )
        );
        // Escaped double quote inside quotes.
        assert_eq!(
            "say\\ \\\"hi\\\" /tmp/file".to_string(),
            desktop_exec_to_command("\"say \\\"hi\\\"\" %f", "/tmp/file".to_string(), false)
        );
        // Escaped backslash inside quotes.
        assert_eq!(
            "a\\\\b /tmp/file".to_string(),
            desktop_exec_to_command("\"a\\\\b\" %f", "/tmp/file".to_string(), false)
        );
        // A backslash before a character that is not escapable per spec is
        // kept literally.
        assert_eq!(
            "a\\\\qb /tmp/file".to_string(),
            desktop_exec_to_command("\"a\\qb\" %f", "/tmp/file".to_string(), false)
        );
        // Escaped dollar sign inside quotes.
        assert_eq!(
            "\\$5 /tmp/file".to_string(),
            desktop_exec_to_command("\"\\$5\" %f", "/tmp/file".to_string(), false)
        );
        // Single quotes are not special per spec; they pass through
        // literally (escaped for the shell).
        assert_eq!(
            "it\\'s /tmp/file".to_string(),
            desktop_exec_to_command("it's %f", "/tmp/file".to_string(), false)
        );
        // An unterminated quote gracefully treats the rest of the string as
        // part of the current argument.
        assert_eq!(
            "foo bar\\ baz /tmp/file".to_string(),
            desktop_exec_to_command("foo \"bar baz", "/tmp/file".to_string(), false)
        );
        // An argument that expands to nothing (an explicitly empty argument
        // or a purged field code) is dropped.
        assert_eq!(
            "viewer /tmp/file".to_string(),
            desktop_exec_to_command("viewer \"\" %f", "/tmp/file".to_string(), false)
        );
    }

    #[test]
    fn test_desktop_exec_shell_metacharacters_in_path() {
        assert_eq!(
            "viewer /tmp/x\\$\\(touch\\ pwned\\)y\\;z".to_string(),
            desktop_exec_to_command("viewer %f", "/tmp/x$(touch pwned)y;z".to_string(), false)
        );
        assert_eq!(
            "viewer /tmp/a\\`id\\`b".to_string(),
            desktop_exec_to_command("viewer %f", "/tmp/a`id`b".to_string(), false)
        );
        // A URL containing a space stays a single argument.
        assert_eq!(
            "browser http://example.com/a\\ b".to_string(),
            desktop_exec_to_command("browser %u", "http://example.com/a b".to_string(), true)
        );
    }

    #[test]
    fn test_parse_desktop_exec_args() {
        assert!(parse_desktop_exec_args("").is_empty());
        assert_eq!(parse_desktop_exec_args("a b\tc"), ["a", "b", "c"]);
        assert_eq!(parse_desktop_exec_args("\"a b\" c"), ["a b", "c"]);
        // Quotes inside a token glue its pieces into a single argument.
        assert_eq!(parse_desktop_exec_args("a\"b c\"d"), ["ab cd"]);
        // An explicit empty quoted argument survives tokenization.
        assert_eq!(parse_desktop_exec_args("\"\" x"), ["", "x"]);
        assert_eq!(parse_desktop_exec_args("x \"y z"), ["x", "y z"]);
        assert_eq!(parse_desktop_exec_args("x\\"), ["x\\"]);
    }

    #[test]
    fn test_desktop_exec_field_codes_preserved() {
        // %i, %c and %k expand to no argument at all.
        assert_eq!(
            "eog /tmp/file".to_string(),
            desktop_exec_to_command("eog %i %f", "/tmp/file".to_string(), false)
        );
        // The first field code in f, F, u, U priority order wins, and only
        // its first occurrence is expanded.
        assert_eq!(
            "foo %F /tmp/file".to_string(),
            desktop_exec_to_command("foo %F %f", "/tmp/file".to_string(), false)
        );
        assert_eq!(
            "foo /tmp/file %f".to_string(),
            desktop_exec_to_command("foo %f %f", "/tmp/file".to_string(), false)
        );
        // Field codes expand inside quoted arguments too, like the previous
        // implementation did (the spec leaves this undefined).
        assert_eq!(
            "v /tmp/file".to_string(),
            desktop_exec_to_command("v \"%f\"", "/tmp/file".to_string(), false)
        );
    }

    #[test]
    fn truncate_middle_short_borrowed() {
        assert!(matches!(truncate_middle("", 128, 240), Cow::Borrowed("")));
        assert_eq!(
            truncate_middle("short subject.txt", 128, 240),
            "short subject.txt"
        );
        assert!(matches!(
            truncate_middle("short subject.txt", 128, 240),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn truncate_middle_ascii_128_kept() {
        let input = "a".repeat(128);
        assert_eq!(truncate_middle(&input, 128, 240), input);
    }

    #[test]
    fn truncate_middle_ascii_200() {
        let input = "a".repeat(200);
        let result = truncate_middle(&input, 128, 240);
        assert_eq!(result.chars().count(), 128);
        assert!(result.starts_with(&input[..62]));
        assert!(result.ends_with(&input[137..]));
        assert_eq!(result.find("..."), Some(62));
    }

    #[test]
    fn truncate_middle_cjk_200() {
        let input = "请".repeat(200);
        let result = truncate_middle(&input, 128, 240);
        assert!(result.chars().count() <= 128);
        assert!(result.len() <= 240);
        assert!(result.contains("..."));
        assert!(result.starts_with('请'));
        assert!(result.ends_with('请'));
    }

    #[test]
    fn truncate_middle_mixed_emoji_cjk() {
        let input = "😀甲请𝄞".repeat(50);
        let result = truncate_middle(&input, 128, 240);
        let Ok(_) = String::from_utf8(result.as_bytes().to_vec()) else {
            panic!("truncate_middle produced invalid UTF-8: {result:?}");
        };
        assert!(result.chars().count() <= 128);
        assert!(result.len() <= 240);
    }

    #[test]
    fn truncate_middle_zero_budget() {
        // Degenerate budgets must not panic; the longest prefix fitting
        // zero chars is the empty string.
        assert!(truncate_middle("abc", 0, 0).is_empty());
    }

    #[test]
    fn truncate_middle_byte_cap_only() {
        // 240 bytes / 80 chars: within both caps, returned untouched.
        let fits = "请".repeat(80);
        assert!(matches!(truncate_middle(&fits, 128, 240), Cow::Borrowed(_)));
        // 243 bytes but still 81 <= 128 chars: only the byte cap trips.
        let one_over = "请".repeat(81);
        let result = truncate_middle(&one_over, 128, 240);
        assert!(result.len() <= 240);
        assert!(result.contains("..."));
    }
}
