//! Right-to-left text for GPUI: what to hand the toolkit so that Arabic, Hebrew, Persian and
//! mixed-direction titles show as Chromium showed them.
//!
//! **Why.** On Windows, GPUI lays a line out with an `IDWriteTextLayout`, which runs the bidi
//! algorithm and shapes each right-to-left run correctly, but GPUI's `IDWriteTextRenderer`
//! (`DrawGlyphRun` in gpui-pre-windows' `direct_write.rs`) ignores the run's origin and its
//! `bidiLevel` and lays every run's glyphs left to right, one after the other, in the order
//! DirectWrite reports them. A right-to-left run's glyphs come in logical order, so each one
//! comes out mirrored: the letters reversed, joined forms facing the wrong way, mark offsets
//! pointing the wrong way. On macOS, CoreText's `CTLine` positions are used as they are, so the
//! order is right there, but the paragraph direction is CoreText's "natural" one (the first
//! strong character), where Chromium's was left to right (the pages set no `dir`).
//!
//! **What this does.** [`display`] takes the logical string (as it comes from the stream, the
//! tags, the file name or the user) and returns what to give GPUI:
//!
//! - Pure left-to-right text (anything without a right-to-left letter or control) comes back
//!   borrowed, untouched, after one pass over the characters (no allocation; an ASCII string is
//!   one `is_ascii` check).
//! - On Windows, a string with right-to-left text comes back in *visual* order: the Unicode Bidi
//!   Algorithm (UAX #9, paragraph left to right like the pages) reorders it, mirrored characters
//!   (brackets) are swapped in right-to-left runs, combining marks stay after their base letter
//!   (runs are reversed by grapheme cluster), and Arabic letters are first converted to their
//!   presentation forms (initial, medial, final, isolated, and the lam-alef ligatures) so that
//!   their joined shapes survive the reordering. A LEFT-TO-RIGHT OVERRIDE in front makes
//!   DirectWrite lay it all out left to right as given, so GPUI's renderer gets only
//!   left-to-right runs and draws them right.
//! - On macOS the string stays logical with a LEFT-TO-RIGHT MARK in front, which makes the
//!   paragraph left to right like the pages; CoreText reorders and shapes it.
//! - With the `native-bidi` feature (only when the app carries the GPUI patch that makes
//!   `DrawGlyphRun` place right-to-left runs where DirectWrite puts them), strings with letters
//!   that have no presentation forms ([`needs_native_shaping`]) take the macOS route too, so
//!   they're shaped by DirectWrite; everything else still takes the visual route, for its
//!   truncation (below).
//!
//! The returned [`Display::direction`] is the string's own direction (its first strong
//! character). The pages never used it: every title was left-aligned (or centered) in a
//! left-to-right paragraph, whatever its script, so don't align by it if the UI must match.
//!
//! **Truncation and the marquee** need nothing more: the visual string truncates at its end with
//! GPUI's `text_ellipsis`, which is its right edge, where Chromium put the ellipsis in a
//! left-to-right paragraph (Chromium cuts what overflows visually, so for an Arabic title it
//! hides the title's *beginning*, at the right, and shows its end; so does this). The marquee
//! measures and scrolls the visual string like any other: it starts showing the left end and
//! sweeps to the right end, as the page did (for an Arabic title the left end is the end of the
//! sentence). A logical string (the macOS route) truncates logically instead: GPUI keeps its
//! beginning, so an overflowing right-to-left title shows its start, not its end, and the
//! ellipsis sits next to it.
//!
//! **Only for display.** Give the logical string, never this one, to tooltips (the Win32 tooltip
//! does its own bidi), accessibility labels, the clipboard, text inputs and anything stored.
//!
//! Limits of the visual route: letters that have no presentation form (Kurdish ڵ ڕ ێ ە, Syriac,
//! N'Ko, Mandaic, Adlam...) are shown unjoined, and their neighbors don't join to them; optional
//! ligatures (other than lam-alef and الله) and kerning between Arabic letters are lost, so
//! Arabic runs can come out a pixel or two narrower than Chromium's. Arabic, Persian and Urdu
//! letters all have presentation forms.

mod forms;

use icu_properties::props::{BidiMirroringGlyph, JoiningType};
use icu_properties::CodePointMapData;
use std::borrow::Cow;
use unicode_bidi::{bidi_class, BidiClass, BidiInfo, Level};
use unicode_segmentation::UnicodeSegmentation;

/// U+202D LEFT-TO-RIGHT OVERRIDE: what follows is laid out left to right as it is.
const LRO: char = '\u{202D}';
/// U+200E LEFT-TO-RIGHT MARK: an invisible strong left-to-right character.
const LRM: char = '\u{200E}';
/// U+200C ZERO WIDTH NON-JOINER.
const ZWNJ: char = '\u{200C}';

/// A paragraph direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

/// What to show for a string, and its own direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display<'a> {
    /// What to hand GPUI: the input itself when it has no right-to-left text.
    pub text: Cow<'a, str>,
    /// The direction of the string's first strong character (left to right when it has none).
    pub direction: Direction,
}

/// What to give GPUI for the logical string `text` on this platform (see the crate docs).
pub fn display(text: &str) -> Display<'_> {
    let direction = direction(text);
    let text = if cfg!(target_os = "macos") || (cfg!(feature = "native-bidi") && needs_native_shaping(text)) {
        logical_ltr(text)
    } else {
        visual(text)
    };
    Display { text, direction }
}

/// Whether `text` has right-to-left letters that join but have no presentation forms (Syriac,
/// N'Ko, Kurdish ڵ ڕ ێ...): only a text system that shapes right-to-left runs itself can show
/// them joined.
pub fn needs_native_shaping(text: &str) -> bool {
    has_rtl(text) && text.chars().any(|c| matches!(bidi_class(c), BidiClass::R | BidiClass::AL) && joins_natively(c) && joining(c) == Joining::None)
}

/// [`display`]'s text for any string type GPUI takes: the same value (for a `SharedString`, the
/// same shared buffer) when there's nothing to do.
pub fn shown<S: AsRef<str> + Clone + From<String>>(text: &S) -> S {
    match display(text.as_ref()).text {
        Cow::Borrowed(_) => text.clone(),
        Cow::Owned(owned) => S::from(owned),
    }
}

/// Whether `text` has anything that reads right to left: a right-to-left letter (Hebrew, Arabic,
/// Syriac, Thaana, N'Ko...) or a right-to-left embedding, override or isolate.
pub fn has_rtl(text: &str) -> bool {
    // Nothing below U+0590 is right to left: Latin, Greek, Cyrillic, Armenian titles stop here.
    !text.is_ascii() && text.chars().any(|c| c >= '\u{0590}' && is_rtl(c))
}

fn is_rtl(c: char) -> bool {
    matches!(bidi_class(c), BidiClass::R | BidiClass::AL | BidiClass::RLE | BidiClass::RLO | BidiClass::RLI)
}

/// The direction of the first strong character outside isolates (UAX #9 P2, P3); left to right
/// when there is none.
pub fn direction(text: &str) -> Direction {
    if !has_rtl(text) {
        return Direction::Ltr;
    }
    let mut isolates = 0usize;
    for c in text.chars() {
        match bidi_class(c) {
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => isolates += 1,
            BidiClass::PDI => isolates = isolates.saturating_sub(1),
            BidiClass::L if isolates == 0 => return Direction::Ltr,
            BidiClass::R | BidiClass::AL if isolates == 0 => return Direction::Rtl,
            _ => {}
        }
    }
    Direction::Ltr
}

/// The logical string with a LEFT-TO-RIGHT MARK in front when it has right-to-left text: for a
/// text system that does bidi itself, a left-to-right paragraph like the pages'.
pub fn logical_ltr(text: &str) -> Cow<'_, str> {
    if !has_rtl(text) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 3);
    out.push(LRM);
    out.push_str(text);
    Cow::Owned(out)
}

/// The visual-order string (see the crate docs) when `text` has right-to-left text.
pub fn visual(text: &str) -> Cow<'_, str> {
    if !has_rtl(text) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() * 2 + 3);
    out.push(LRO);
    out.extend(visual_chars(text).into_iter().map(|(c, _)| c));
    Cow::Owned(out)
}

/// For each character of [`visual`]'s output after the override, the index (in characters) of
/// the logical character it shows (a lam-alef ligature shows its lam; an inserted non-joiner
/// shows the character before it). For tests and for mapping positions back.
pub fn visual_map(text: &str) -> Vec<usize> {
    if !has_rtl(text) {
        return (0..text.chars().count()).collect();
    }
    visual_chars(text).into_iter().map(|(_, i)| i).collect()
}

fn visual_chars(text: &str) -> Vec<(char, usize)> {
    let logical: Vec<char> = text.chars().collect();
    let shaped = shape_arabic(&logical);
    let string: String = shaped.iter().map(|&(c, _)| c).collect();
    // Byte offset in `string` → index in `shaped`.
    let mut at_byte = vec![usize::MAX; string.len() + 1];
    for (i, (byte, _)) in string.char_indices().enumerate() {
        at_byte[byte] = i;
    }

    let info = BidiInfo::new(&string, Some(Level::ltr()));
    let mirror = CodePointMapData::<BidiMirroringGlyph>::new();
    let mut out: Vec<(char, usize)> = Vec::with_capacity(shaped.len() + 4);
    for paragraph in &info.paragraphs {
        let (levels, runs) = info.visual_runs(paragraph, paragraph.range.clone());
        for run in runs {
            let rtl = levels[run.start].is_rtl();
            let piece = &string[run.clone()];
            if !rtl {
                for (byte, c) in piece.char_indices() {
                    if !is_control(c) {
                        out.push((c, shaped[at_byte[run.start + byte]].1));
                    }
                }
                continue;
            }
            // Right to left: clusters in reverse, each in its own order, mirrored.
            let mut previous_joins = false;
            for (offset, cluster) in piece.grapheme_indices(true).rev() {
                let mut chars = cluster.char_indices().filter(|&(_, c)| !is_control(c)).peekable();
                let Some(&(_, first)) = chars.peek() else { continue };
                // Two letters that still join by themselves (no presentation form) would be
                // joined by the shaper in this reversed order, the wrong way round: keep them
                // apart.
                let joins = joins_natively(first);
                if joins && previous_joins {
                    if let Some(&(_, index)) = out.last() {
                        out.push((ZWNJ, index));
                    }
                }
                previous_joins = joins;
                for (byte, c) in chars {
                    let glyph = mirror.get(c);
                    let c = if glyph.mirrored { glyph.mirroring_glyph.unwrap_or(c) } else { c };
                    out.push((c, shaped[at_byte[run.start + offset + byte]].1));
                }
            }
        }
    }
    out
}

/// Explicit embeddings, overrides and isolates: resolved by the algorithm, then dropped (one left
/// in would end or undo the override in front).
fn is_control(c: char) -> bool {
    matches!(
        bidi_class(c),
        BidiClass::LRE | BidiClass::RLE | BidiClass::LRO | BidiClass::RLO | BidiClass::PDF | BidiClass::LRI | BidiClass::RLI | BidiClass::FSI | BidiClass::PDI
    )
}

/// A letter the shaper would still join to its neighbors (it was left as it is).
fn joins_natively(c: char) -> bool {
    let joining = CodePointMapData::<JoiningType>::new().get(c);
    c != '\u{200D}' && matches!(joining, JoiningType::DualJoining | JoiningType::RightJoining | JoiningType::LeftJoining | JoiningType::JoinCausing)
}

/// How a character joins once shaped here.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Joining {
    /// Doesn't join (or has no presentation forms to show joined).
    None,
    /// A combining mark: skipped when looking for neighbors.
    Transparent,
    /// Tatweel, ZWJ: joins on both sides, has no forms of its own.
    Causing,
    /// Joins on both sides.
    Dual,
    /// Joins to the previous letter only.
    Right,
}

impl Joining {
    // Can join to the character before it (on its right, in Arabic).
    fn joins_previous(self) -> bool {
        matches!(self, Joining::Dual | Joining::Right | Joining::Causing)
    }

    // Can join to the character after it.
    fn joins_next(self) -> bool {
        matches!(self, Joining::Dual | Joining::Causing)
    }
}

#[derive(Clone, Copy)]
struct Forms {
    isolated: u32,
    final_: u32,
    initial: u32,
    medial: u32,
}

fn forms(c: char) -> Option<Forms> {
    let code = c as u32;
    forms::FORMS
        .binary_search_by_key(&code, |f| f.0)
        .ok()
        .map(|i| forms::FORMS[i])
        .map(|(_, isolated, final_, initial, medial)| Forms { isolated, final_, initial, medial })
}

fn joining(c: char) -> Joining {
    match CodePointMapData::<JoiningType>::new().get(c) {
        JoiningType::Transparent => Joining::Transparent,
        JoiningType::JoinCausing => Joining::Causing,
        JoiningType::DualJoining | JoiningType::RightJoining | JoiningType::LeftJoining => match forms(c) {
            // What can be shown decides how it joins: a dual-joining letter without initial and
            // medial forms is shown as if right-joining.
            Some(f) if f.final_ != 0 && f.initial != 0 && f.medial != 0 => Joining::Dual,
            Some(f) if f.final_ != 0 => Joining::Right,
            _ => Joining::None,
        },
        _ => Joining::None,
    }
}

fn lam_alef(alef: char) -> Option<(u32, u32)> {
    forms::LAM_ALEF.iter().find(|l| l.0 == alef as u32).map(|l| (l.1, l.2))
}

fn to_char(code: u32, fallback: char) -> char {
    char::from_u32(code).filter(|_| code != 0).unwrap_or(fallback)
}

/// Arabic contextual shaping in logical order: each letter replaced by the presentation form
/// for its position (UAX #9-independent: joining is between logical neighbors, skipping marks),
/// lam + alef by their ligature. Returns the characters with their logical indices.
fn shape_arabic(chars: &[char]) -> Vec<(char, usize)> {
    let identity = || chars.iter().copied().zip(0..).collect();
    if !chars.iter().any(|&c| ('\u{0600}'..='\u{08FF}').contains(&c)) {
        return identity();
    }
    let mut types: Vec<Joining> = chars.iter().map(|&c| joining(c)).collect();
    // The word الله on its own (no marks, not joined on either side) is one ligature in Segoe UI
    // (as Chromium shows it): U+FDF2.
    let mut allah = vec![false; chars.len()];
    for i in 0..chars.len().saturating_sub(3) {
        if chars[i..i + 4] != ['\u{0627}', '\u{0644}', '\u{0644}', '\u{0647}'] {
            continue;
        }
        let before = (0..i).rev().find(|&k| types[k] != Joining::Transparent).map(|k| types[k]);
        let after = (i + 4..chars.len()).find(|&k| types[k] != Joining::Transparent).map(|k| types[k]);
        if !before.is_some_and(Joining::joins_next) && !after.is_some_and(Joining::joins_previous) {
            allah[i] = true;
            types[i..i + 4].fill(Joining::None);
        }
    }
    // The alef each lam ligates with: the ligature joins to what precedes the lam, like a
    // right-joining letter, and the alef is consumed.
    let mut ligature: Vec<Option<usize>> = vec![None; chars.len()];
    for i in 0..chars.len() {
        if chars[i] != '\u{0644}' {
            continue;
        }
        let Some(j) = (i + 1..chars.len()).find(|&j| types[j] != Joining::Transparent) else { continue };
        if lam_alef(chars[j]).is_some() {
            ligature[i] = Some(j);
            types[i] = Joining::Right;
        }
    }
    let previous = |i: usize, types: &[Joining]| (0..i).rev().find(|&k| types[k] != Joining::Transparent).map(|k| types[k]);
    let next = |i: usize, types: &[Joining]| (i + 1..chars.len()).find(|&k| types[k] != Joining::Transparent).map(|k| types[k]);

    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let t = types[i];
        if allah[i] {
            out.push(('\u{FDF2}', i));
            i += 4;
            continue;
        }
        let joins_previous = t.joins_previous() && previous(i, &types).is_some_and(|p| p.joins_next());
        if let Some(j) = ligature[i] {
            let (isolated, final_) = lam_alef(chars[j]).unwrap_or((0, 0));
            out.push((to_char(if joins_previous { final_ } else { isolated }, c), i));
            // The marks between lam and alef follow the ligature.
            out.extend((i + 1..j).map(|k| (chars[k], k)));
            i = j + 1;
            continue;
        }
        let shown = match (t, forms(c)) {
            (Joining::Dual | Joining::Right, Some(f)) => {
                let joins_next = t.joins_next() && next(i, &types).is_some_and(|n| n.joins_previous());
                let code = match (joins_previous, joins_next) {
                    (true, true) => f.medial,
                    (true, false) => f.final_,
                    (false, true) => f.initial,
                    (false, false) => f.isolated,
                };
                to_char(code, c)
            }
            _ => c,
        };
        out.push((shown, i));
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // The visual string without the override, as code points, for readable failures.
    fn v(text: &str) -> String {
        let shown = visual(text);
        assert!(shown.starts_with(LRO), "{text:?} wasn't reordered");
        shown[LRO.len_utf8()..].to_string()
    }

    fn rev(s: &str) -> String {
        s.chars().rev().collect()
    }

    #[test]
    fn left_to_right_is_untouched_and_borrowed() {
        for text in ["", "Daft Punk - Around the World", "Café Ŝtrasse", "Ελληνικά Кириллица", "周杰伦 - 晴天", "Chill 🎧 ☕", "١٢:٣٤"] {
            assert!(matches!(visual(text), Cow::Borrowed(_)), "{text}");
            assert!(matches!(logical_ltr(text), Cow::Borrowed(_)), "{text}");
            assert_eq!(direction(text), Direction::Ltr);
        }
        let shared = String::from("KEXP");
        assert_eq!(shown(&shared), "KEXP");
    }

    #[test]
    fn hebrew_is_reversed() {
        assert_eq!(v("שלום"), rev("שלום"));
        assert_eq!(direction("שלום"), Direction::Rtl);
        // Spaces between Hebrew words are part of the run.
        assert_eq!(v("עומר אדם"), rev("עומר אדם"));
    }

    #[test]
    fn numbers_inside_right_to_left_keep_their_order() {
        // "כאן 88": the digits stay 88, to the left of the Hebrew word.
        assert_eq!(v("כאן 88"), format!("88 {}", rev("כאן")));
        // Persian digits are European numbers too.
        assert_eq!(v("ת ۲۴"), "۲۴ ת");
        // Arabic-Indic digits with a separator.
        assert_eq!(v("ס ١٢:٣٤"), "١٢:٣٤ ס");
    }

    #[test]
    fn mixed_runs_in_a_left_to_right_paragraph() {
        // Latin first: the Hebrew run is reversed in place, the paragraph stays left to right.
        assert_eq!(v("Omer Adam - תגידו לה"), format!("Omer Adam - {}", rev("תגידו לה")));
        // Hebrew first: still left to right, so the Hebrew comes first, on the left.
        assert_eq!(v("עומר אדם - Live"), format!("{} - Live", rev("עומר אדם")));
        assert_eq!(direction("עומר אדם - Live"), Direction::Rtl);
        assert_eq!(direction("Live - עומר"), Direction::Ltr);
    }

    #[test]
    fn neutrals_at_the_boundaries_follow_the_paragraph() {
        // The dash and spaces between two Hebrew words are right to left; the ones between Hebrew
        // and Latin take the paragraph's direction; a trailing "!" too.
        assert_eq!(v("אב - גד"), rev("אב - גד"));
        assert_eq!(v("אב!"), format!("{}!", rev("אב")));
        assert_eq!(v("!אב"), format!("!{}", rev("אב")));
    }

    #[test]
    fn brackets_are_mirrored_in_right_to_left_runs() {
        // "אב (גד) הו": one run; reversed, the parentheses swap so they still open towards גד.
        assert_eq!(v("אב (גד) הו"), "וה (דג) בא");
        // Parentheses around Latin inside a left-to-right paragraph are left alone.
        assert_eq!(v("אב (Live)"), format!("{} (Live)", rev("אב")));
        // Square brackets around a number in Hebrew.
        assert_eq!(v("אב [2]"), "[2] בא");
    }

    #[test]
    fn combining_marks_stay_after_their_base() {
        // שָׁ = shin, qamats, shin dot: one cluster, kept in order.
        let shown = v("שָׁלוֹם");
        let clusters: Vec<&str> = shown.graphemes(true).collect();
        assert_eq!(clusters, vec!["ם", "וֹ", "ל", "שָׁ"]);
    }

    #[test]
    fn arabic_letters_get_their_joined_forms() {
        // فيروز: feh initial, yeh medial, reh final, waw isolated, zain isolated; then reversed.
        assert_eq!(v("فيروز"), "\u{FEAF}\u{FEED}\u{FEAE}\u{FEF4}\u{FED3}");
        // A lone letter is isolated.
        assert_eq!(v("ب"), "\u{FE8F}");
        // Marks are transparent: بِسْمِ joins through the kasra and sukun.
        let shown = v("بِسْمِ");
        assert_eq!(shown.chars().filter(|c| !('\u{064B}'..='\u{0652}').contains(c)).collect::<String>(), "\u{FEE2}\u{FEB4}\u{FE91}");
        assert_eq!(shown.graphemes(true).collect::<Vec<_>>(), vec!["\u{FEE2}\u{0650}", "\u{FEB4}\u{0652}", "\u{FE91}\u{0650}"]);
    }

    #[test]
    fn persian_and_urdu_letters() {
        // پ (peh) گ (gaf) چ (tcheh) ی (farsi yeh): all have forms. "پگی" → peh initial, gaf
        // medial, farsi yeh final.
        assert_eq!(v("پگی"), "\u{FBFD}\u{FB95}\u{FB58}");
        // ے (yeh barree) only joins to the previous letter.
        assert_eq!(v("بے"), "\u{FBAF}\u{FE91}");
    }

    #[test]
    fn lam_alef_ligatures() {
        // لا alone: the isolated ligature. سلا: seen initial, then the final ligature.
        assert_eq!(v("لا"), "\u{FEFB}");
        assert_eq!(v("سلا"), "\u{FEFC}\u{FEB3}");
        // لإ, and a mark between lam and alef moves after the ligature.
        assert_eq!(v("لإ"), "\u{FEF9}");
        assert_eq!(v("لَا"), "\u{FEFB}\u{064E}");
    }

    #[test]
    fn the_word_allah() {
        // On its own: the ligature, like Segoe UI's own.
        assert_eq!(v("الله"), "\u{FDF2}");
        assert_eq!(v("لا إله إلا الله").chars().next(), Some('\u{FDF2}'));
        // و doesn't join forward: still on its own.
        assert_eq!(v("والله"), "\u{FDF2}\u{FEED}");
        // Joined to what follows (اللهم), or with marks: spelled out.
        assert!(!v("اللهم").contains('\u{FDF2}'));
        assert!(!v("اللَّه").contains('\u{FDF2}'));
        // Joined to what precedes (بالله): spelled out, the alef final.
        let shown = v("بالله");
        assert!(!shown.contains('\u{FDF2}') && shown.contains('\u{FE8E}'), "{shown}");
    }

    #[test]
    fn non_joining_neighbors() {
        // Alef doesn't join forward: باب = beh initial, alef final, beh isolated.
        assert_eq!(v("باب"), "\u{FE8F}\u{FE8E}\u{FE91}");
        // ZWNJ breaks a join: ب‌ب = two isolated behs.
        assert_eq!(v("ب\u{200C}ب").replace(ZWNJ, ""), "\u{FE8F}\u{FE8F}");
        // Tatweel joins on both sides and stays itself.
        assert_eq!(v("بـب"), "\u{FE90}\u{0640}\u{FE91}");
    }

    #[test]
    fn already_presentation_forms_are_only_reordered() {
        let legacy = "\u{FED3}\u{FEF4}\u{FEAE}\u{FEED}\u{FEAF}";
        assert_eq!(v(legacy), rev(legacy));
    }

    #[test]
    fn letters_without_forms_are_kept_apart() {
        // Syriac has no presentation forms: shown unjoined, in visual order, with non-joiners
        // between letters so the shaper doesn't join them the wrong way round.
        let shown = v("ܫܠܡܐ");
        assert_eq!(shown.replace(ZWNJ, ""), rev("ܫܠܡܐ"));
        assert_eq!(shown.matches(ZWNJ).count(), 3);
        // Kurdish ڵ (lam with small v) has none either; the beh before it doesn't join to it.
        assert_eq!(v("بڵ").replace(ZWNJ, ""), "\u{06B5}\u{FE8F}");
    }

    #[test]
    fn mixed_arabic_and_latin_with_numbers() {
        // "عمرو دياب - Tamally Maak (Remix 2024)": the Arabic run is first, on the left.
        let shown = v("عمرو دياب - Tamally Maak (Remix 2024)");
        assert!(shown.ends_with(" - Tamally Maak (Remix 2024)"), "{shown}");
        // "أم كلثوم - أنت عمري (تسجيل حي 1964) [الجزء 2]": one right-to-left run; the numbers keep
        // their digits' order and the brackets are mirrored.
        let shown = v("أم كلثوم - أنت عمري (تسجيل حي 1964) [الجزء 2]");
        assert!(shown.starts_with("[2 "), "{shown}");
        assert!(shown.contains("(1964 "), "{shown}");
        assert!(!shown.contains('\u{0644}') && !shown.contains('\u{0627}'), "every letter shaped: {shown}");
    }

    #[test]
    fn controls_are_resolved_and_dropped() {
        // An RLO in the input reverses Latin text; the control itself is gone.
        assert_eq!(v("\u{202E}abc\u{202C} d"), "cba d");
        // An isolate around Hebrew in a Latin title.
        assert_eq!(v("Song \u{2067}שיר\u{2069} 2"), format!("Song {} 2", rev("שיר")));
        assert_eq!(direction("\u{2067}שיר\u{2069} Song"), Direction::Ltr);
    }

    #[test]
    fn emoji_sequences_stay_whole_inside_right_to_left() {
        // A ZWJ emoji between Hebrew words: the sequence is one cluster, kept in order.
        let shown = v("אב 👩🏽\u{200D}💻 גד");
        assert!(shown.contains("👩🏽\u{200D}💻"), "{shown}");
        assert_eq!(shown.replace("👩🏽\u{200D}💻", "X"), "דג X בא");
    }

    #[test]
    fn map_points_back_to_logical_characters() {
        assert_eq!(visual_map("ab"), vec![0, 1]);
        assert_eq!(visual_map("אב c"), vec![1, 0, 2, 3]);
        // The ligature maps to the lam, the alef is gone.
        assert_eq!(visual_map("سلا"), vec![1, 0]);
    }

    #[test]
    fn what_needs_native_shaping() {
        assert!(needs_native_shaping("ܫܠܡܐ"));
        assert!(needs_native_shaping("گوڵی"));
        assert!(!needs_native_shaping("فيروز - كيفك إنت"));
        assert!(!needs_native_shaping("پخش زنده ـ"));
        assert!(!needs_native_shaping("שלום"));
        assert!(!needs_native_shaping("Mongolian ᠮᠣᠩᠭᠣᠯ"));
    }

    #[test]
    fn logical_ltr_adds_a_mark() {
        assert_eq!(logical_ltr("שלום"), format!("{LRM}שלום"));
    }
}
