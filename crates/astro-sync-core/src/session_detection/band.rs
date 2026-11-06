//! The band of a night read from names, when the frames carry no
//! `FILTER`.
//!
//! Astro Sync tells two bands only: broadband and dual-band. When a header
//! names no filter, the person is asked; before asking, the names around the
//! frames are read for a word that says it (`180s_dualband`, `RGB`,
//! `UVIR_cut`…). Sources, in order: the folders from the night's own up to
//! the location's root, the frame's file name, the first heading of a
//! `SESSION.md` (or `README.md`) beside the frames or above them. A guess is
//! only ever a preselection: the person sees where it comes from and can
//! change it.

use serde::{Deserialize, Serialize};

/// The two bands the tool tells apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NameBand {
    Broadband,
    Dualband,
}

/// Where a guess was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuessOrigin {
    /// A folder of the night (or above it).
    Folder,
    /// The frame's own file name.
    File,
    /// The first heading of a `SESSION.md` / `README.md`.
    Note,
}

/// The band a night's names say, and the words that said it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BandGuess {
    pub band: NameBand,
    pub origin: GuessOrigin,
    /// What the person is shown: the folder or file name (without a leading
    /// date), or the heading of the note.
    pub source: String,
    /// The note's file name (`SESSION.md`), for [`GuessOrigin::Note`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_file: Option<String>,
    /// The filter the words name (`Dualband`, `UV/IR cut`, `RGB`…), when they
    /// name one: what a light without a `FILTER` card is given as its filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_name: Option<String>,
}

/// Words of a filter that lets two narrow bands through (normalised: lower
/// case, words joined by `_`). Each is a real filter or a plain word for it.
const DUAL: &[&str] = &[
    "dualband",
    "dual_band",
    "duoband",
    "duo_band",
    "duo",
    "bibande",
    "bi_bande",
    "biband",
    "bi_band",
    "l_extreme",
    "lextreme",
    "l_enhance",
    "lenhance",
    "l_ultimate",
    "lultimate",
    "alp_t",
    "alpt",
    "nbz",
    "ha_oiii",
    "haoiii",
    "hoo",
];

/// Words of a filter or a set-up that is broadband for certain.
const BROAD_STRONG: &[&str] = &[
    "broadband",
    "broad_band",
    "wideband",
    "wide_band",
    "largebande",
    "large_bande",
    "uvir",
    "uv_ir",
    "ircut",
    "ir_cut",
    "cut",
    "cls",
    "lps",
    "none",
    "nofilter",
    "no_filter",
];

/// Words that name a colour camera's usual frames (no filter in front of
/// them is no proof): a dual-band word beside them wins.
const BROAD_WEAK: &[&str] = &["rgb", "osc"];

/// The text with its words split at every non-alphanumeric character and
/// joined by `_`, lower-cased; `camel` also splits `DualBand` at the capital.
fn normalised(text: &str, camel: bool) -> String {
    let mut out = String::from("_");
    let mut previous: Option<char> = None;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if camel && c.is_uppercase() && previous.is_some_and(char::is_lowercase) {
                push_separator(&mut out);
            }
            out.extend(c.to_lowercase());
        } else {
            push_separator(&mut out);
        }
        previous = Some(c);
    }
    push_separator(&mut out);
    out
}

fn push_separator(out: &mut String) {
    if !out.ends_with('_') {
        out.push('_');
    }
}

/// Whether `word` (normalised) stands as whole words in `haystack`.
fn holds(haystack: &str, word: &str) -> bool {
    haystack.contains(&format!("_{word}_"))
}

/// The band one text says, if it says exactly one: a dual-band word wins
/// over `RGB` / `OSC` alone, and a dual-band word beside a broadband filter
/// word is no answer (the text contradicts itself).
#[must_use]
pub fn band_in(text: &str) -> Option<NameBand> {
    let plain = normalised(text, false);
    let split = normalised(text, true);
    let found = |words: &[&str]| {
        words
            .iter()
            .any(|word| holds(&plain, word) || holds(&split, word))
    };
    let dual = found(DUAL);
    let strong = found(BROAD_STRONG);
    let weak = found(BROAD_WEAK);
    match (dual, strong, weak) {
        (true, false, _) => Some(NameBand::Dualband),
        (false, true, _) | (false, false, true) => Some(NameBand::Broadband),
        _ => None,
    }
}

/// The words that name a filter of the broadband family, with the name given
/// to it (checked in this order; normalised words as above).
const BROAD_FILTERS: &[(&[&str], &str)] = &[
    (&["uvir", "uv_ir", "uv_ir_cut", "uvir_cut"], "UV/IR cut"),
    (&["ircut", "ir_cut"], "IR cut"),
    (&["cls"], "CLS"),
    (&["lps"], "LPS"),
    (&["rgb"], "RGB"),
];

/// The filter one text names, when its band is told (`band_in`) and the words
/// name a filter: `Dualband` for any dual-band word, then the broadband
/// filter words (`UV/IR cut`, `IR cut`, `CLS`, `LPS`, `RGB`). `None` for a
/// text that says a band without naming a filter (`nofilter`, `OSC`).
#[must_use]
pub fn filter_name_in(text: &str) -> Option<String> {
    let band = band_in(text)?;
    if band == NameBand::Dualband {
        return Some("Dualband".to_string());
    }
    let plain = normalised(text, false);
    let split = normalised(text, true);
    BROAD_FILTERS
        .iter()
        .find(|(words, _)| {
            words
                .iter()
                .any(|word| holds(&plain, word) || holds(&split, word))
        })
        .map(|(_, name)| (*name).to_string())
}

/// A folder or file name as shown: the leading date (`2026-08-09_10_`) is
/// dropped, the rest is what the person named.
#[must_use]
pub fn shown_name(name: &str) -> String {
    let bytes = name.as_bytes();
    let digits = |from: usize, count: usize| {
        bytes.len() >= from + count && bytes[from..from + count].iter().all(u8::is_ascii_digit)
    };
    let is_separator = |index: usize| matches!(bytes.get(index), Some(b'-' | b'_'));
    if digits(0, 4) && is_separator(4) && digits(5, 2) && is_separator(7) && digits(8, 2) {
        let mut end = 10;
        // « _10 »: the morning's day of a night crossing midnight.
        if is_separator(end) && digits(end + 1, 2) && !digits(end + 3, 1) {
            end += 3;
        }
        let rest = name[end..].trim_start_matches(['_', '-', ' ']);
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    name.to_string()
}

/// The first heading (`# …`) of a note, in its first lines.
#[must_use]
pub fn first_heading(note: &str) -> Option<String> {
    note.lines()
        .take(40)
        .map(str::trim)
        .find(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|heading| !heading.is_empty())
}

/// What the names around one frame say: the folders (the deepest, the
/// night's own, first), then the file name, then the notes (`(file name,
/// first heading)`, the nearest first).
#[must_use]
pub fn guess_for_frame(
    folders: &[String],
    file_name: &str,
    notes: &[(String, String)],
) -> Option<BandGuess> {
    for folder in folders.iter().rev() {
        if let Some(band) = band_in(folder) {
            return Some(BandGuess {
                band,
                origin: GuessOrigin::Folder,
                source: shown_name(folder),
                note_file: None,
                filter_name: filter_name_in(folder),
            });
        }
    }
    if let Some(band) = band_in(file_name) {
        return Some(BandGuess {
            band,
            origin: GuessOrigin::File,
            source: file_name.to_string(),
            note_file: None,
            filter_name: filter_name_in(file_name),
        });
    }
    notes.iter().find_map(|(file, heading)| {
        band_in(heading).map(|band| BandGuess {
            band,
            origin: GuessOrigin::Note,
            source: heading.clone(),
            note_file: Some(file.clone()),
            filter_name: filter_name_in(heading),
        })
    })
}

/// The night's guess from its frames' guesses: one band only (two bands in
/// one night is a question, not a guess); the most common words say it.
#[must_use]
pub fn guess_for_night<'a>(guesses: impl IntoIterator<Item = &'a BandGuess>) -> Option<BandGuess> {
    let mut counted: Vec<(&BandGuess, usize)> = Vec::new();
    for guess in guesses {
        match counted.iter_mut().find(|(seen, _)| *seen == guess) {
            Some((_, count)) => *count += 1,
            None => counted.push((guess, 1)),
        }
    }
    let band = counted.first()?.0.band;
    if counted.iter().any(|(guess, _)| guess.band != band) {
        return None;
    }
    // The first met wins a tie.
    counted
        .iter()
        .fold(None::<&(&BandGuess, usize)>, |best, item| match best {
            Some(best) if best.1 >= item.1 => Some(best),
            _ => Some(item),
        })
        .map(|(guess, _)| (*guess).clone())
}

#[cfg(test)]
mod tests {
    use super::NameBand::{Broadband, Dualband};
    use super::*;

    #[test]
    fn the_names_of_the_three_real_nights_say_their_band() {
        assert_eq!(band_in("2026-08-09_10_180s_dualband"), Some(Dualband));
        assert_eq!(band_in("2026-08-10_11_90s_RGB"), Some(Broadband));
        assert_eq!(band_in("2026-08-12_120s_UVIR_cut"), Some(Broadband));
    }

    #[test]
    fn the_headings_of_the_three_real_notes_say_their_band() {
        assert_eq!(
            band_in("SH2-190 — 9/10 août 2026 — dual-band 180 s"),
            Some(Dualband)
        );
        assert_eq!(
            band_in("SH2-190 — 10/11 août 2026 — RGB 90 s"),
            Some(Broadband)
        );
        assert_eq!(
            band_in("SH2-190 — 12 août 2026 — UV/IR-cut 120 s"),
            Some(Broadband)
        );
    }

    #[test]
    fn tricky_names() {
        // A target name around a colour word.
        assert_eq!(band_in("RGB_SH2-190_Voile"), Some(Broadband));
        assert_eq!(band_in("UVIR_cut"), Some(Broadband));
        assert_eq!(band_in("Ha-OIII"), Some(Dualband));
        assert_eq!(band_in("HaOIII_180s"), Some(Dualband));
        assert_eq!(band_in("ha oiii"), Some(Dualband));
        assert_eq!(band_in("L-eXtreme"), Some(Dualband));
        assert_eq!(band_in("L-eNhance 180s"), Some(Dualband));
        assert_eq!(band_in("Night_DualBand"), Some(Dualband));
        assert_eq!(band_in("dual band"), Some(Dualband));
        assert_eq!(band_in("Dual-Band"), Some(Dualband));
        assert_eq!(band_in("ALP-T"), Some(Dualband));
        assert_eq!(band_in("NBZ"), Some(Dualband));
        assert_eq!(band_in("HOO_palette"), Some(Dualband));
        assert_eq!(band_in("nofilter"), Some(Broadband));
        assert_eq!(band_in("none"), Some(Broadband));
        assert_eq!(band_in("IRcut"), Some(Broadband));
        assert_eq!(band_in("CLS"), Some(Broadband));
        assert_eq!(band_in("OSC_L-Pro_lps"), Some(Broadband));
        // A name with none of them.
        assert_eq!(band_in("SH2-190_Voile_M27"), None);
        assert_eq!(band_in("2026-08-09_10_180s"), None);
        assert_eq!(band_in("lights"), None);
        // Whole words only.
        assert_eq!(band_in("Oscar_Duomo_Hoodie_Cutoff_Clsx"), None);
        assert_eq!(
            band_in("Light_M31_180.0s_Bin1_533MC_gain100_-10.0C_0001"),
            None
        );
    }

    #[test]
    fn a_dual_band_word_beats_rgb_but_not_a_broadband_filter() {
        assert_eq!(band_in("OSC_L-eXtreme"), Some(Dualband));
        assert_eq!(band_in("RGB_dualband"), Some(Dualband));
        // The text contradicts itself: no guess.
        assert_eq!(band_in("UVIR_dualband"), None);
        assert_eq!(band_in("lps_ha-oiii"), None);
    }

    #[test]
    fn the_names_of_the_three_real_nights_name_their_filter() {
        assert_eq!(
            filter_name_in("2026-08-09_10_180s_dualband").as_deref(),
            Some("Dualband")
        );
        assert_eq!(
            filter_name_in("2026-08-10_11_90s_RGB").as_deref(),
            Some("RGB")
        );
        assert_eq!(
            filter_name_in("2026-08-12_120s_UVIR_cut").as_deref(),
            Some("UV/IR cut")
        );
        assert_eq!(
            filter_name_in("SH2-190 — 12 août 2026 — UV/IR-cut 120 s").as_deref(),
            Some("UV/IR cut")
        );
        assert_eq!(
            filter_name_in("SH2-190 — 9/10 août 2026 — dual-band 180 s").as_deref(),
            Some("Dualband")
        );
        // A band without a filter in it, or no band at all.
        assert_eq!(filter_name_in("nofilter"), None);
        assert_eq!(filter_name_in("OSC_session"), None);
        assert_eq!(filter_name_in("lights"), None);
        // A text that contradicts itself names nothing.
        assert_eq!(filter_name_in("UVIR_dualband"), None);
        assert_eq!(filter_name_in("RGB_dualband").as_deref(), Some("Dualband"));
    }

    #[test]
    fn a_name_is_shown_without_its_date() {
        assert_eq!(shown_name("2026-08-09_10_180s_dualband"), "180s_dualband");
        assert_eq!(shown_name("2026-08-12_120s_UVIR_cut"), "120s_UVIR_cut");
        assert_eq!(shown_name("2026-08-09"), "2026-08-09");
        assert_eq!(shown_name("RGB_SH2-190_Voile"), "RGB_SH2-190_Voile");
        // « _120s » is not a day.
        assert_eq!(shown_name("2026-08-12_120s_x"), "120s_x");
    }

    #[test]
    fn the_first_heading_of_a_note_is_read() {
        assert_eq!(
            first_heading("\n# SH2-190 — dual-band 180 s\n\n- 100 lights\n").as_deref(),
            Some("SH2-190 — dual-band 180 s")
        );
        assert_eq!(first_heading("no heading\n- RGB"), None);
        assert_eq!(first_heading("##  \ntext"), None);
    }

    #[test]
    fn the_folders_come_first_then_the_file_then_the_note() {
        let folders: Vec<String> = ["SH2-190", "2026-08-09_10_180s_dualband", "lights"]
            .map(String::from)
            .to_vec();
        let notes = vec![("SESSION.md".to_string(), "SH2-190 — RGB 90 s".to_string())];
        let guess = guess_for_frame(&folders, "Light_RGB_0001.fit", &notes).unwrap();
        assert_eq!(guess.band, Dualband);
        assert_eq!(guess.origin, GuessOrigin::Folder);
        assert_eq!(guess.source, "180s_dualband");

        let bare: Vec<String> = ["SH2-190", "night", "lights"].map(String::from).to_vec();
        let guess = guess_for_frame(&bare, "Light_RGB_0001.fit", &notes).unwrap();
        assert_eq!((guess.band, guess.origin), (Broadband, GuessOrigin::File));
        let notes = vec![("SESSION.md".to_string(), "SH2-190 — dual-band".to_string())];
        let guess = guess_for_frame(&bare, "Light_0001.fit", &notes).unwrap();
        assert_eq!((guess.band, guess.origin), (Dualband, GuessOrigin::Note));
        assert_eq!(guess.note_file.as_deref(), Some("SESSION.md"));
        assert_eq!(guess_for_frame(&bare, "Light_0001.fit", &[]), None);
    }

    #[test]
    fn the_deepest_folder_wins_over_its_parents() {
        let folders: Vec<String> = ["RGB_archive", "2026-08-09_dualband", "lights"]
            .map(String::from)
            .to_vec();
        let guess = guess_for_frame(&folders, "a.fit", &[]).unwrap();
        assert_eq!(guess.band, Dualband);
    }

    #[test]
    fn a_night_with_two_bands_is_no_guess() {
        let make = |band, source: &str| BandGuess {
            band,
            origin: GuessOrigin::Folder,
            source: source.to_string(),
            note_file: None,
            filter_name: None,
        };
        let a = make(Dualband, "dualband");
        let b = make(Dualband, "dualband");
        let c = make(Broadband, "RGB");
        assert_eq!(guess_for_night([&a, &b]).unwrap().band, Dualband);
        assert_eq!(guess_for_night([&a, &c]), None);
        assert_eq!(guess_for_night(std::iter::empty()), None);
    }
}
