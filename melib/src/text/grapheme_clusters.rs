/*
 * meli - text mod.
 *
 * Copyright 2017-2020 Manos Pitsidianakis
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

//! Breaks a string into individual user-perceived "characters".
//!
//! Unicode UAX-29 standard, version 10.0.0

use unicode_segmentation::UnicodeSegmentation;

use super::{types::Reflow, wcwidth::wcwidth};

/// Whether a `base` + `U+FE0F` (emoji presentation) cluster renders two
/// columns wide in a terminal.
///
/// True for a non-ASCII base that measures a single column: East Asian
/// Ambiguous / text-default symbols such as `☑` (U+2611) or `⌨` (U+2328)
/// are rendered two columns wide once the emoji presentation selector
/// follows them, even though [`wcwidth`] reports one. ASCII bases (`#`,
/// digits, letters) are not emoji-capable on their own — `#\u{FE0F}`
/// without the enclosing keycap `U+20E3` stays one column — so they are
/// excluded.
///
/// This is the single source of truth for such a cluster's width:
/// [`TextProcessing::grapheme_width`] measures with it and
/// `CellBuffer::write_string` (in meli) reserves the continuation cell
/// with it, so layout math and grid accounting cannot drift apart.
pub const fn is_emoji_presentation_base(base: char) -> bool {
    !base.is_ascii() && matches!(wcwidth(base), Some(1))
}

pub trait TextProcessing: UnicodeSegmentation + AsRef<str> {
    /// Returns a vector containing each grapheme as a slice.
    fn split_graphemes(&self) -> Vec<&str> {
        UnicodeSegmentation::graphemes(self, true).collect::<Vec<&str>>()
    }

    /// Returns a vector containing each grapheme and the index it starts at as a
    /// slice.
    fn graphemes_indices(&self) -> Vec<(usize, &str)> {
        UnicodeSegmentation::grapheme_indices(self, true).collect::<Vec<(usize, &str)>>()
    }

    /// Returns the first grapheme and the zero index.
    fn next_grapheme(&self) -> Option<(usize, &str)> {
        UnicodeSegmentation::grapheme_indices(self, true).next()
    }

    /// Returns the last grapheme and its starting index.
    fn last_grapheme(&self) -> Option<(usize, &str)> {
        UnicodeSegmentation::grapheme_indices(self, true).next_back()
    }

    /// Returns the total display width of the string: the sum of
    /// [`wcwidth`] over its code-points, except that a `base` + `U+FE0F`
    /// (emoji presentation) cluster counts as two columns when its base
    /// is an emoji-capable single-column symbol (see
    /// [`is_emoji_presentation_base`]).
    fn grapheme_width(&self) -> usize {
        use unicode_width::UnicodeWidthStr;
        // Delegate to the unicode-width crate's CJK-context string-level
        // calculation (Ambiguous = 2, Wide = 2, letters narrow, emoji ZWJ
        // ligatures and presentation sequences handled atomically), then
        // correct for terminal-UI characters that CJK mode over-widens.
        //
        // Box-drawing (U+2500..U+257F) and block elements (U+2580..U+259F)
        // are the border/gauge vocabulary of every terminal UI; they
        // render one column on all terminals including CJK-locale ones,
        // but width_cjk measures the Ambiguous ones as two. Leaving them
        // at two doubled every border line and scrollbar glyph, pushing
        // content past the viewport edge.
        //
        // Correction is per-grapheme-cluster (not per-char) so that ZWJ
        // emoji ligatures keep their atomic two-column width from
        // width_cjk's string-level pass.
        let s = self.as_ref();
        let raw = s.width_cjk();
        // Fast path: pure ASCII never contains wide characters.
        if raw == s.len() {
            return raw;
        }
        let mut correction = 0;
        for g in self.split_graphemes() {
            let g_cjk = {
                use unicode_width::UnicodeWidthStr;
                g.width_cjk()
            };
            // Only box-drawing / block-element clusters need narrowing;
            // everything else keeps the string-level width_cjk result.
            if g_cjk > 1 && g.chars().all(|c| matches!(c as u32, 0x2500..=0x259F)) {
                correction += g_cjk - 1;
            }
        }
        raw.saturating_sub(correction)
    }

    /// Returns the amount of graphemes.
    fn grapheme_len(&self) -> usize {
        self.split_graphemes().len()
    }

    /// Splits lines at word boundaries without breaking any words, for given
    /// line width.
    fn split_lines(&self, width: usize) -> Vec<String>;

    /// Splits lines at word boundaries without breaking any words, for given
    /// line width, using a reflow algorithm to balance line width
    /// variation.
    fn split_lines_reflow(&self, reflow: Reflow, width: Option<usize>) -> Vec<String>;
}

impl TextProcessing for str {
    fn split_lines(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![];
        }
        super::line_break::linear(self, width)
    }

    fn split_lines_reflow(&self, reflow: Reflow, width: Option<usize>) -> Vec<String> {
        if width == Some(0) {
            return vec![];
        }
        super::line_break::split_lines_reflow(self, reflow, width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::TextPresentation;

    #[test]
    fn box_drawing_stays_narrow() {
        // Terminal-UI border characters render one column on all
        // terminals, including CJK-locale ones.
        assert_eq!("\u{2500}".grapheme_width(), 1); // ─ light horizontal
        assert_eq!("\u{2502}".grapheme_width(), 1); // │ light vertical
        assert_eq!("\u{2551}".grapheme_width(), 1); // ║ double vertical
        assert_eq!("\u{2588}".grapheme_width(), 1); // █ full block
                                                    // A border line of 10 chars is 10 columns, not 20.
        assert_eq!("──────────".grapheme_width(), 10);
    }

    #[test]
    fn test_grapheme_width() {
        assert_eq!("●".grapheme_width(), 2);
        assert_eq!("●📎".grapheme_width(), 4);
        assert_eq!("●📎︎".grapheme_width(), 4);
        assert_eq!("●\u{FE0E}📎\u{FE0E}".grapheme_width(), 4);
        assert_eq!("🎃".grapheme_width(), 2);
        assert_eq!("👻".grapheme_width(), 2);
        assert_eq!("🛡︎".grapheme_width(), 1); // text presentation → narrow
        assert_eq!("🛡︎".text_pr().grapheme_width(), 1); // text presentation → narrow

        assert_eq!("こんにちわ世界".grapheme_width(), 14);
        assert_eq!("こ★ん■に●ち▲わ☆世◆界".grapheme_width(), 26);
    }

    /// `base` + `U+FE0F` clusters occupy two columns when the base is an
    /// emoji-capable single-column symbol, and stay one column for ASCII
    /// bases (no keycap) and two columns for wide bases — the same rule
    /// `CellBuffer::write_string`'s `U+FE0F` branch applies.
    #[test]
    fn test_grapheme_width_emoji_presentation() {
        // Text-default symbol + FE0F: two columns.
        assert_eq!("\u{2611}\u{FE0F}".grapheme_width(), 2);
        assert_eq!("\u{2328}\u{FE0F}".grapheme_width(), 2);
        // Wide base + FE0F: already two columns, not four.
        assert_eq!("\u{1F4CE}\u{FE0F}".grapheme_width(), 2);
        // ASCII base + FE0F stays one column.
        assert_eq!("a\u{FE0F}".grapheme_width(), 1);
        // Surrounding text keeps its own width.
        assert_eq!("x\u{2611}\u{FE0F}y".grapheme_width(), 4);
        // Ballot box: narrow (not East-Asian Ambiguous).
        assert_eq!("\u{2611}".grapheme_width(), 1);
    }
}
