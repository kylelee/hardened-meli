/*
 * meli - melib crate.
 *
 * Copyright 2017-2020 Manos Pitsidianakis
 * Copyright 2026 Kyle Lee
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

#![expect(clippy::needless_range_loop)]

include!("src/text/types.rs");

use std::{
    fs::File,
    io::prelude::*,
    path::Path,
    process::{Command, Stdio},
};

const MOD_PATH: &str = "src/text/tables.rs";

/* Pinning scheme for the UCD data files (CWE-829 hardening):
 *
 * - The generated tables committed at `MOD_PATH` act as the cache: as long as
 *   that file exists, the build script never touches the network and its
 *   integrity is guaranteed by version control.  The download path below only
 *   runs when the tables are (re)generated, i.e. when `MOD_PATH` is absent.
 * - Downloads go over HTTPS only (`curl --proto '=https'`) and each file's
 *   SHA-256 is checked against a pinned digest below.  An unknown version, a
 *   digest mismatch or a fetch failure aborts the build; unverified data is
 *   never used.
 *
 * Regenerating/rotating the pins: fetch the four files for the new version
 * over HTTPS (e.g. `curl --fail --proto '=https' -O <url>`), record the
 * output of `sha256sum` for each, and add or update the corresponding entry
 * in `PINNED_UCD_DIGESTS`. */

/// Pinned SHA-256 digests of the UCD input files, per Unicode version.
///
/// Digest order per entry: `LineBreak.txt`, `UnicodeData.txt`,
/// `EastAsianWidth.txt`, `emoji-data.txt`.
const PINNED_UCD_DIGESTS: &[(&str, [&str; 4])] = &[(
    "16.0.0",
    [
        // LineBreak.txt
        "e97e4259d0d20fab150b9c7b4b28abfae5cd78ca97e7f4ac6ed20d685d5f4a7c",
        // UnicodeData.txt
        "ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f",
        // EastAsianWidth.txt
        "43adc76c0686a42cb370764eb8cfe2b2a45b10b855e5572a2db4a0eecce15d5b",
        // emoji-data.txt
        "f1365a5173eee18e1f98b240cdc492e84a25f1ce7e0c9d1094eb29c41a22696a",
    ],
)];

fn pinned_digests(version: &str) -> Option<[&'static str; 4]> {
    PINNED_UCD_DIGESTS
        .iter()
        .find(|(v, _)| *v == version)
        .map(|(_, digests)| *digests)
}

/* Minimal SHA-256 (FIPS 180-4), hand-rolled because melib deliberately has no
 * [build-dependencies] (no `sha2` crate available here).  It is only ever
 * executed inside this build script against build-verified data, and its own
 * correctness is asserted against NIST test vectors on every build script
 * invocation via `sha256_self_check` (see its call at the top of `main`). */
mod sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    pub(crate) fn digest(data: &[u8]) -> [u8; 32] {
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];

        // data || 0x80 || zero padding || 64-bit big-endian bit length
        let bit_len = u64::try_from(data.len())
            .expect("message longer than 2^64 - 1 bits")
            .wrapping_mul(8);
        let mut message = Vec::with_capacity(data.len() + 72);
        message.extend_from_slice(data);
        message.push(0x80);
        while message.len() % 64 != 56 {
            message.push(0);
        }
        message.extend_from_slice(&bit_len.to_be_bytes());

        for offset in (0..message.len()).step_by(64) {
            let block: &[u8; 64] = message[offset..offset + 64].try_into().unwrap();
            let mut w = [0_u32; 64];
            for i in 0..16 {
                let word: [u8; 4] = block[4 * i..4 * i + 4].try_into().unwrap();
                w[i] = u32::from_be_bytes(word);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            let [na, nb, nc, nd, ne, nf, ng, nh] = h;
            h = [
                na.wrapping_add(a),
                nb.wrapping_add(b),
                nc.wrapping_add(c),
                nd.wrapping_add(d),
                ne.wrapping_add(e),
                nf.wrapping_add(f),
                ng.wrapping_add(g),
                nh.wrapping_add(hh),
            ];
        }

        let mut out = [0_u8; 32];
        for i in 0..8 {
            out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
        }
        out
    }

    pub(crate) fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

/// Assert the hand-rolled SHA-256 against NIST FIPS 180-4 test vectors
/// (expected values cross-checked with coreutils `sha256sum`).
///
/// Runs on every build script invocation before anything else so a broken
/// hasher can never silently approve tampered data; hashing these few hundred
/// bytes is negligible.
fn sha256_self_check() {
    const VECTORS: &[(&str, &str)] = &[
        // Empty message.
        (
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        // "abc" (single block).
        (
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        // 56-byte message (padding pushes it to two blocks).
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        // 103-byte message (spans two blocks).
        (
            concat!(
                "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmno",
                "ijklmnopjklmnopqklmnoprlmnopsmnopetnopu"
            ),
            "ec6d792a0bf0ba2a8d241955c16a5e89459595a22fa489e3a876de03b0d271b1",
        ),
    ];
    for (input, expected) in VECTORS {
        let got = sha256::hex(&sha256::digest(input.as_bytes()));
        assert_eq!(
            &got, expected,
            "SHA-256 self-check failed for input {input:?}: got {got}, expected {expected}"
        );
    }
}

/// Download `url` over HTTPS with curl, failing hard on any error.
fn curl(url: &str) -> Result<Vec<u8>, std::io::Error> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--max-time",
            "600",
            "--output",
            "-",
            url,
        ])
        .stdout(Stdio::piped())
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "curl exited with status {} while fetching {url}",
            output.status
        )));
    }
    Ok(output.stdout)
}

#[derive(Debug)]
struct Ucd {
    line_break_table: String,
    unicode_data: String,
    east_asian_width: String,
    emoji_data: String,
}

impl Ucd {
    fn get(version: &str) -> Result<Self, std::io::Error> {
        let Some(digests) = pinned_digests(version) else {
            panic!(
                "Unicode version {version:?} has no pinned SHA-256 digests for its UCD files; \
                 refusing to download and trust unpinned data. To add it, fetch the four files \
                 from https://www.unicode.org/Public/{version}/ucd/ over HTTPS, record their \
                 sha256sum digests and add them to PINNED_UCD_DIGESTS in build.rs."
            );
        };

        let fetch = |path: &str, digest: &str| -> Result<String, std::io::Error> {
            let url = format!("https://www.unicode.org/Public/{version}/{path}");
            let contents = curl(&url)?;
            let actual = sha256::hex(&sha256::digest(&contents));
            if actual != digest {
                panic!(
                    "SHA-256 mismatch for {url}\n  expected (pinned): {digest}\n  actual: \
                     {actual}\nThe downloaded UCD data is not the pinned one; aborting. If this \
                     is an intentional rotation, update PINNED_UCD_DIGESTS in build.rs."
                );
            }
            String::from_utf8(contents).map_err(|err| {
                std::io::Error::other(format!("fetched {url} is not valid UTF-8: {err}"))
            })
        };

        Ok(Self {
            line_break_table: fetch("ucd/LineBreak.txt", digests[0])?,
            unicode_data: fetch("ucd/UnicodeData.txt", digests[1])?,
            east_asian_width: fetch("ucd/EastAsianWidth.txt", digests[2])?,
            emoji_data: fetch("ucd/emoji/emoji-data.txt", digests[3])?,
        })
    }
}

fn main() -> Result<(), std::io::Error> {
    sha256_self_check();
    let version: String = std::env::var("UNICODE_VERSION").unwrap_or("16.0.0".into());
    println!("cargo:rerun-if-env-changed=UNICODE_REGENERATE_TABLES");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={MOD_PATH}");

    eprintln!("Fetching unicode data tables for unicode version {version:?}");
    let mod_path = Path::new(MOD_PATH);
    if mod_path.exists() {
        eprintln!(
            "{} already exists, delete it if you want to replace it.",
            mod_path.display()
        );
        return Ok(());
    }
    let ucd = Ucd::get(&version).expect("failed to fetch and verify UCD data files");
    let mut line_break_table: Vec<(u32, u32, LineBreakClass)> = Vec::with_capacity(3800);
    for line in ucd.line_break_table.lines() {
        if line.starts_with('#') || line.starts_with(' ') || line.is_empty() {
            continue;
        }
        let mut fields: [&str; 2] = line.split(';').collect::<Vec<&str>>().try_into().unwrap();
        fields[0] = fields[0].trim();
        fields[1] = fields[1].trim();

        /* LineBreak.txt list is ascii encoded so we can assume each char takes one
         * byte: */

        let mut codepoint_iter = fields[0].split("..");

        let first_codepoint: u32 = u32::from_str_radix(codepoint_iter.next().unwrap(), 16).unwrap();

        let sec_codepoint: u32 = codepoint_iter
            .next()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .unwrap_or(first_codepoint);
        let class = fields[1]
            .trim()
            .split('#')
            .next()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        line_break_table.push((first_codepoint, sec_codepoint, LineBreakClass::from(class)));
    }

    const MAX_CODEPOINT: usize = 0x110000;
    // See https://www.unicode.org/L2/L1999/UnicodeData.html
    const FIELD_CODEPOINT: usize = 0;
    const FIELD_CATEGORY: usize = 2;
    // Ambiguous East Asian characters
    const WIDTH_AMBIGUOUS_EASTASIAN: isize = -3;

    // Width changed from 1 to 2 in Unicode 9.0
    const WIDTH_WIDENED_IN_9: isize = -6;
    // Category for unassigned codepoints.
    const CAT_UNASSIGNED: &str = "Cn";

    // Category for private use codepoints.
    const CAT_PRIVATE_USE: &str = "Co";

    // Category for surrogates.
    const CAT_SURROGATE: &str = "Cs";

    struct Codepoint<'cat> {
        raw: u32,
        width: Option<isize>,
        category: &'cat str,
    }

    let mut codepoints: Vec<Codepoint> = Vec::with_capacity(MAX_CODEPOINT + 1);
    for i in 0..=MAX_CODEPOINT {
        codepoints.push(Codepoint {
            raw: i as u32,
            width: None,
            category: CAT_UNASSIGNED,
        });
    }

    set_general_categories(&mut codepoints, &ucd.unicode_data);
    set_eaw_widths(&mut codepoints, &ucd.east_asian_width);
    set_emoji_widths(&mut codepoints, &ucd.emoji_data);
    set_hardcoded_ranges(&mut codepoints);
    fn hexrange_to_range(hexrange: &str) -> std::ops::Range<usize> {
        /* Given a string like 1F300..1F320 representing an inclusive range,
        return the range of codepoints.
        If the string is like 1F321, return a range of just that element.
        */
        let hexrange = hexrange.trim();
        let fields = hexrange
            .split("..")
            .map(|h| usize::from_str_radix(h.trim(), 16).unwrap())
            .collect::<Vec<usize>>();
        if fields.len() == 1 {
            fields[0]..(fields[0] + 1)
        } else {
            fields[0]..(fields[1] + 1)
        }
    }

    fn set_general_categories<'u>(codepoints: &mut [Codepoint<'u>], unicode_data: &'u str) {
        for line in unicode_data.lines() {
            let fields = line.trim().split(';').collect::<Vec<_>>();
            if fields.len() > FIELD_CATEGORY {
                for idx in hexrange_to_range(fields[FIELD_CODEPOINT]) {
                    codepoints[idx].category = fields[FIELD_CATEGORY];
                }
            }
        }
    }

    fn set_eaw_widths(codepoints: &mut [Codepoint<'_>], eaw_data_lines: &str) {
        //  Read from EastAsianWidth.txt, set width values on the codepoints
        for line in eaw_data_lines.lines() {
            let line = line.trim().split('#').next().unwrap_or(line);
            let fields = line.trim().split(';').collect::<Vec<_>>();
            if fields.len() != 2 {
                continue;
            }
            let hexrange = fields[0].trim();
            let width_type = fields[1].trim();
            // width_types:
            //  A: ambiguous, F: fullwidth, H: halfwidth,
            // . N: neutral, Na: east-asian Narrow
            let width: isize = if width_type == "A" {
                WIDTH_AMBIGUOUS_EASTASIAN
            } else if width_type == "F" || width_type == "W" {
                2
            } else {
                1
            };
            for cp in hexrange_to_range(hexrange) {
                codepoints[cp].width = Some(width);
            }
        }
        // Apply the following special cases:
        //  - The unassigned code points in the following blocks default to "W":
        //         - CJK Unified Ideographs Extension A: U+3400..U+4DBF
        //         - CJK Unified Ideographs:             U+4E00..U+9FFF
        //         - CJK Compatibility Ideographs:       U+F900..U+FAFF
        //  - All undesignated code points in Planes 2 and 3, whether inside or outside
        //    of allocated blocks, default to "W":
        //         - Plane 2:                            U+20000..U+2FFFD
        //         - Plane 3:                            U+30000..U+3FFFD
        const WIDE_RANGES: [(usize, usize); 5] = [
            (0x3400, 0x4DBF),
            (0x4E00, 0x9FFF),
            (0xF900, 0xFAFF),
            (0x20000, 0x2FFFD),
            (0x30000, 0x3FFFD),
        ];
        for &wr in WIDE_RANGES.iter() {
            for cp in wr.0..(wr.1 + 1) {
                if codepoints[cp].width.is_none() {
                    codepoints[cp].width = Some(2);
                }
            }
        }
    }

    fn set_emoji_widths(codepoints: &mut [Codepoint<'_>], emoji_data_lines: &str) {
        // Read from emoji-data.txt, set codepoint widths
        for line in emoji_data_lines.lines() {
            if !line.contains('#') || line.trim().starts_with('#') {
                continue;
            }
            let mut fields = line.trim().split('#').collect::<Vec<_>>();
            if fields.len() != 2 {
                continue;
            }
            let comment = fields.pop().unwrap();
            let fields = fields.pop().unwrap();

            let hexrange = fields.split(';').next().unwrap();

            // In later versions of emoji-data.txt there are some "reserved"
            // entries that have "NA" instead of a Unicode version number
            // of first use, they will now return a zero version instead of
            // crashing the script
            if comment.trim().starts_with("NA") {
                continue;
            }

            use std::str::FromStr;
            let mut v = comment.split_whitespace().next().unwrap();
            if v.starts_with('E') {
                v = &v[1..];
            }
            if v.as_bytes()
                .first()
                .map(|c| !c.is_ascii_digit())
                .unwrap_or(true)
            {
                continue;
            }
            let mut idx = 1;
            while v
                .as_bytes()
                .get(idx)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                idx += 1;
            }
            if v.as_bytes().get(idx).map(|&c| c != b'.').unwrap_or(true) {
                continue;
            }
            idx += 1;
            while v
                .as_bytes()
                .get(idx)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                idx += 1;
            }
            v = &v[0..idx];

            let version = f32::from_str(v).unwrap();
            for cp in hexrange_to_range(hexrange) {
                // Don't consider <=1F000 values as emoji. These can only be made
                // emoji through the variation selector which interacts terribly
                // with wcwidth().
                if cp < 0x1F000 {
                    continue;
                }
                // Skip codepoints that are explicitly not wide.
                // For example U+1F336 ("Hot Pepper") renders like any emoji but is
                // marked as neutral in EAW so has width 1 for some reason.
                //if codepoints[cp].width == Some(1) {
                //    continue;
                //}

                // If this emoji was introduced before Unicode 9, then it was widened in 9.
                codepoints[cp].width = if version >= 9.0 {
                    Some(2)
                } else {
                    Some(WIDTH_WIDENED_IN_9)
                };
            }
        }
    }
    fn set_hardcoded_ranges(codepoints: &mut [Codepoint<'_>]) {
        // Mark private use and surrogate codepoints
        // Private use can be determined awkwardly from UnicodeData.txt,
        // but we just hard-code them.
        // We do not treat "private use high surrogate" as private use
        // so as to match wcwidth9().
        const PRIVATE_RANGES: [(usize, usize); 3] =
            [(0xE000, 0xF8FF), (0xF0000, 0xFFFFD), (0x100000, 0x10FFFD)];
        for &(first, last) in PRIVATE_RANGES.iter() {
            for idx in first..=last {
                codepoints[idx].category = CAT_PRIVATE_USE;
            }
        }

        const SURROGATE_RANGES: [(usize, usize); 2] = [(0xD800, 0xDBFF), (0xDC00, 0xDFFF)];
        for &(first, last) in SURROGATE_RANGES.iter() {
            for idx in first..=last {
                codepoints[idx].category = CAT_SURROGATE;
            }
        }
    }

    let mut file = File::create(mod_path)?;
    file.write_all(
        br#"//
// meli
//
// Copyright 2017- Manos Pitsidianakis <manos@pitsidianak.is>
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

"#,
    )
    .unwrap();
    file.write_all(format!("//! Generated for Unicode version {version}").as_bytes())
        .unwrap();
    file.write_all(
        br#"

use super::types::LineBreakClass::{self, *};

pub const LINE_BREAK_RULES: &[(u32, u32, LineBreakClass)] = &[
"#,
    )
    .unwrap();
    for l in &line_break_table {
        file.write_all(format!("    (0x{:X}, 0x{:X}, {:?}),\n", l.0, l.1, l.2).as_bytes())
            .unwrap();
    }
    file.write_all(b"];\n").unwrap();

    for (name, filter) in [
        (
            "ASCII",
            Box::new(|c: &&Codepoint| c.raw < 0x7f && c.raw >= 0x20)
                as Box<dyn Fn(&&Codepoint) -> bool>,
        ),
        (
            "PRIVATE",
            Box::new(|c: &&Codepoint| c.category == CAT_PRIVATE_USE),
        ),
        (
            "NONPRINT",
            Box::new(|c: &&Codepoint| {
                ["Cc", "Cf", "Zl", "Zp", CAT_SURROGATE].contains(&c.category)
            }),
        ),
        (
            "COMBINING",
            Box::new(|c: &&Codepoint| ["Mn", "Mc", "Me"].contains(&c.category)),
        ),
        ("DOUBLEWIDE", Box::new(|c: &&Codepoint| c.width == Some(2))),
        (
            "UNASSIGNED",
            Box::new(|c: &&Codepoint| c.category == CAT_UNASSIGNED),
        ),
        (
            "AMBIGUOUS",
            Box::new(|c: &&Codepoint| c.width == Some(WIDTH_AMBIGUOUS_EASTASIAN)),
        ),
        (
            "WIDENEDIN9",
            Box::new(|c: &&Codepoint| c.width == Some(WIDTH_WIDENED_IN_9)),
        ),
    ]
    .iter()
    {
        file.write_all(
            format!(
                r#"
pub const {name}: &[(u32, u32)] = &[
"#
            )
            .as_bytes(),
        )
        .unwrap();
        let mut iter = codepoints.iter().filter(filter);
        if let Some(prev) = iter.next() {
            let mut prev = prev.raw;
            let mut a = prev;
            for cp in iter {
                if prev + 1 != cp.raw {
                    file.write_all(format!("    (0x{a:X}, 0x{prev:X}),\n").as_bytes())
                        .unwrap();
                    a = cp.raw;
                }
                prev = cp.raw;
            }
            file.write_all(format!("    (0x{a:X}, 0x{prev:X}),\n").as_bytes())
                .unwrap();
        }
        file.write_all(b"];\n").unwrap();
    }
    Ok(())
}
