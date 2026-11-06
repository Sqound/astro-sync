//! Unicode NFC for the names Astro Sync compares and writes (names are
//! compared in Unicode NFC, while macOS gives NFD). The Finder hands back "Col des E\u{301}toiles" where
//! the app wrote "Col des Étoiles": both must name the same folder.
//!
//! Without a dependency (none is added to the core for this), the canonical
//! composition runs over a table generated from the Unicode Character
//! Database (`nfc_table.rs`) for the scripts a night's or a target's name is
//! written in: Latin, Greek, Cyrillic and kana. A mark that composes with
//! nothing is kept as it came. A name already in NFC comes back unchanged.

use crate::nfc_table::COMPOSITIONS;

/// The combining marks the table composes: the general combining
/// diacritics and the kana voiced and semi-voiced sound marks.
fn is_combining_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{3099}' | '\u{309A}')
}

fn compose(starter: char, mark: char) -> Option<char> {
    COMPOSITIONS
        .binary_search_by(|(a, b, _)| (*a, *b).cmp(&(starter, mark)))
        .ok()
        .map(|index| COMPOSITIONS[index].2)
}

/// `text` in Unicode NFC (canonical composition) for the scripts above.
#[must_use]
pub fn to_nfc(text: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(text.len());
    // The last starter and whether a mark that did not compose stands after
    // it (a later mark is then blocked from it).
    let mut starter: Option<usize> = None;
    let mut blocked = false;
    for c in text.chars() {
        if is_combining_mark(c) {
            if let Some(index) = starter
                && !blocked
                && let Some(composed) = compose(out[index], c)
            {
                out[index] = composed;
                continue;
            }
            blocked = true;
            out.push(c);
        } else {
            starter = Some(out.len());
            blocked = false;
            out.push(c);
        }
    }
    out.into_iter().collect()
}

/// Two names equal once both are in NFC, ignoring letter case (the default
/// file systems of macOS and Windows ignore it, so two names that differ
/// only by case are one folder there).
#[must_use]
pub fn same_name(a: &str, b: &str) -> bool {
    to_nfc(a).to_lowercase() == to_nfc(b).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_what_macos_hands_back_in_nfd() {
        assert_eq!(to_nfc("Col des E\u{301}toiles"), "Col des Étoiles");
        assert_eq!(to_nfc("Ma\u{308}rz"), "März");
        // Two marks on one letter: ạ then ậ.
        assert_eq!(to_nfc("a\u{323}\u{302}"), "\u{1EAD}");
        // Cyrillic й, kana が and パ.
        assert_eq!(to_nfc("\u{438}\u{306}"), "й");
        assert_eq!(to_nfc("\u{304B}\u{3099}"), "が");
        assert_eq!(to_nfc("\u{30CF}\u{309A}"), "パ");
    }

    #[test]
    fn leaves_nfc_text_and_lonely_marks_alone() {
        assert_eq!(
            to_nfc("NGC 7000 · Col des Étoiles"),
            "NGC 7000 · Col des Étoiles"
        );
        assert_eq!(to_nfc("\u{301}x"), "\u{301}x");
        assert_eq!(to_nfc("q\u{301}"), "q\u{301}");
    }

    #[test]
    fn same_name_ignores_the_form_and_the_case() {
        assert!(same_name("col des e\u{301}toiles", "Col des Étoiles"));
        assert!(!same_name("M31", "M33"));
    }

    #[test]
    fn the_table_is_sorted_for_the_binary_search() {
        assert!(
            COMPOSITIONS
                .windows(2)
                .all(|pair| (pair[0].0, pair[0].1) < (pair[1].0, pair[1].1))
        );
    }
}
