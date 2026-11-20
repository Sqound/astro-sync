//! The reference star list of the measured sky background: Tycho-2, and the
//! one way this crate fetches it (Tycho-2 is bundled).
//!
//! Tycho-2 (Høg et al. 2000, A&A 355; ESA; ≈ 2.5 million stars to V ≈ 11.5)
//! is published by the CDS in VizieR as catalogue `I/259`. The existing
//! mechanism (`photometry::ReferenceStarList`, the `ASRSTAR1` file,
//! `astro-sync reference-stars`) is unchanged: this module adds the identity
//! of the catalogue, the credit that is shown with it, a reader of the
//! tab-separated answer of VizieR's query service, and the download itself,
//! which only runs behind a [`Tycho2Consent`] (the person's agreement,
//! naming the catalogue: see `app/src/i18n`, `sky.referenceStars.*`).
//!
//! Nothing is bundled: the list (≈ 25 MB compact) is not in the repository
//! nor in the installer; it is fetched once, on the person's request, and
//! kept next to the settings as `reference-stars.bin`.
//!
//! Where it comes from, in this order: the project's own download host, as
//! one file whose SHA-256 is checked ([`Tycho2Mirror`]); then VizieR at the
//! CDS; then the other official VizieR installations ([`VIZIER_MIRRORS`]).
//! A source that answers with a page instead of the table, with an error, or
//! with a table that is cut short is a failure and the next source is tried
//! from the first band: bands of two installations are never mixed in one
//! list. With the default depth the finished list must also have the number
//! of stars and the SHA-256 recorded in [`TYCHO2_EXPECTED`].
//! While [`TYCHO2_MIRROR`] is `None` (no file has been published), the
//! download host is skipped.
//!
//! Licence: the catalogue is distributed under CC BY-NC 3.0 IGO with the
//! credit "ESA" (ESA Hipparcos and Tycho catalogues page; ReadMe of I/259 in
//! VizieR): attribution is required and commercial use is not covered. Both
//! credits are shown with the measured value (`TYCHO2.credit`).

use super::photometry::{ReferenceStar, ReferenceStarList, tycho2_v_mag};

/// What a reference catalogue is, for the interface and the agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceCatalogue {
    /// Stable identifier, written in the dataset notes.
    pub id: &'static str,
    pub name: &'static str,
    /// The CDS VizieR catalogue and table it is read from.
    pub vizier_table: &'static str,
    /// The query service.
    pub vizier_base_url: &'static str,
    /// The credit shown with a value measured against it.
    pub credit: &'static str,
    /// Its terms, in one line.
    pub licence: &'static str,
    /// About how many megabytes the compact list weighs.
    pub list_megabytes: u32,
}

pub const TYCHO2: ReferenceCatalogue = ReferenceCatalogue {
    id: "tycho-2",
    name: "Tycho-2",
    vizier_table: "I/259/tyc2",
    vizier_base_url: "https://vizier.cds.unistra.fr/viz-bin/asu-tsv",
    credit: "Tycho-2 Catalogue (Høg et al. 2000, A&A 355, L27; Credit: ESA), via the CDS, VizieR I/259",
    licence: "CC BY-NC 3.0 IGO (ESA): attribution required (Credit: ESA), non-commercial use only",
    list_megabytes: 25,
};

/// The compact list (`ASRSTAR1`, [`DEFAULT_FAINTEST`]) as the project's
/// download host keeps it: one file, named after its own SHA-256, so that a
/// copy that is not the expected one can never be taken for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tycho2Mirror {
    pub url: &'static str,
    /// Hex SHA-256 of the file, lower case.
    pub sha256: &'static str,
    /// Size of the file in bytes.
    pub bytes: u64,
}

/// The published copy of the list, or `None` while there is none (VizieR is
/// then the only source). Set it, with the file's real digest and size, once
/// the file is on the download host.
pub const TYCHO2_MIRROR: Option<Tycho2Mirror> = None;

/// The other official installations of VizieR that serve the same table, in
/// the order they are tried after the CDS (list of the VizieR home page at
/// the CDS: IDIA, CfA Harvard, NAOJ, China-VO). Two of them answer in plain
/// HTTP only; nothing personal is sent and the finished list is checked
/// against [`TYCHO2_EXPECTED`], so they come last.
pub const VIZIER_MIRRORS: [&str; 4] = [
    "https://vizier.cfa.harvard.edu/viz-bin/asu-tsv",
    "https://vizier.idia.ac.za/viz-bin/asu-tsv",
    "http://vizier.nao.ac.jp/viz-bin/asu-tsv",
    "http://vizier.china-vo.org/viz-bin/asu-tsv",
];

/// What the finished list of the default depth must be, whatever the
/// source: its number of stars and the SHA-256 of its compact bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tycho2Expected {
    pub stars: usize,
    /// Hex SHA-256, lower case.
    pub sha256: &'static str,
}

/// The list at [`DEFAULT_FAINTEST`] built from the CfA Harvard installation
/// of VizieR and read back identical from a second run; the answers of the
/// IDIA and NAOJ installations for two bands matched it line for line.
pub const TYCHO2_EXPECTED: Option<Tycho2Expected> = Some(Tycho2Expected {
    stars: 2_411_301,
    sha256: "ff31361274addd539cfeabfeb6594312bd9de3a2410a3f10c6296ca293789840",
});

/// The faintest star kept by default, V magnitude (Tycho-2 stops near 11.5).
pub const DEFAULT_FAINTEST: f64 = 12.5;

/// The person's agreement to fetch Tycho-2. It cannot be made by accident:
/// the interface builds it from a button that names the catalogue, the
/// command line from `--accept-download`.
#[derive(Debug, Clone, Copy)]
pub struct Tycho2Consent(());

impl Tycho2Consent {
    #[must_use]
    pub fn given() -> Self {
        Self(())
    }
}

/// Why the download stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum ReferenceDownloadError {
    /// A request failed; the message is the transport's.
    Network(String),
    /// The answer was not VizieR's table (no header with the columns).
    UnexpectedAnswer(String),
    /// The list was complete but is not the expected Tycho-2 (star count or
    /// digest); the text names the source.
    NotExpected(String),
    /// Nothing usable came back at all.
    Empty,
}

impl std::fmt::Display for ReferenceDownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(message) => write!(f, "{message}"),
            Self::UnexpectedAnswer(band) => {
                write!(f, "unexpected answer from VizieR for the band {band}")
            }
            Self::NotExpected(source) => {
                write!(f, "the list from {source} is not the expected Tycho-2")
            }
            Self::Empty => write!(f, "no star came back"),
        }
    }
}

impl std::error::Error for ReferenceDownloadError {}

/// Width of one declination band, degrees: each request stays a few
/// megabytes, so a slow link or a service limit never loses the whole list.
const BAND_DEGREES: i32 = 5;

/// The declination bands `(low, high)` the list is fetched in, south to north.
#[must_use]
pub fn declination_bands() -> Vec<(i32, i32)> {
    (-90..90)
        .step_by(BAND_DEGREES as usize)
        .map(|low| (low, low + BAND_DEGREES))
        .collect()
}

/// The VizieR query of one band: the columns [`parse_tycho2_vizier_tsv`]
/// reads, stars brighter than `faintest` in `VTmag`, the observed
/// declination (`DE(ICRS)`, which every star has, unlike the mean one)
/// inside the band.
#[must_use]
pub fn band_url(base: &str, low: i32, high: i32, faintest: f64) -> String {
    format!(
        "{base}?-source={}&-out.max=unlimited&-out=RAmdeg&-out=DEmdeg&-out=BTmag&-out=VTmag&-out=RA(ICRS)&-out=DE(ICRS)&VTmag=%3C{faintest}&DE(ICRS)={low}..{high}",
        TYCHO2.vizier_table
    )
}

/// The names VizieR gives a column: today's first, the catalogue's older
/// ones after.
const MEAN_RA: [&str; 2] = ["RAmdeg", "mRAdeg"];
const MEAN_DEC: [&str; 2] = ["DEmdeg", "mDEdeg"];
const OBSERVED_RA: [&str; 2] = ["RA(ICRS)", "RAdeg"];
const OBSERVED_DEC: [&str; 2] = ["DE(ICRS)", "DEdeg"];

fn column(header: &[&str], names: &[&str]) -> Option<usize> {
    names.iter().find_map(|name| {
        header
            .iter()
            .position(|candidate| candidate.trim() == *name)
    })
}

fn number(fields: &[&str], index: Option<usize>) -> Option<f64> {
    fields.get(index?)?.trim().parse::<f64>().ok()
}

/// A star of an answer, and the declination its band was asked by.
struct BandRow {
    star: ReferenceStar,
    band_dec: f64,
}

/// The rows of one VizieR tab-separated answer (see
/// [`parse_tycho2_vizier_tsv`]). `None` when the text has no header with the
/// columns, or when a row is cut short (a table that stops in the middle of a
/// line is not trusted).
fn parse_rows(text: &str) -> Option<Vec<BandRow>> {
    if !text.ends_with('\n') {
        return None;
    }
    let mut lines = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty());
    let header: Vec<&str> = lines.next()?.split('\t').collect();
    let (m_ra, m_dec) = (column(&header, &MEAN_RA), column(&header, &MEAN_DEC));
    let (ra, dec) = (
        column(&header, &OBSERVED_RA),
        column(&header, &OBSERVED_DEC),
    );
    let (bt, vt) = (column(&header, &["BTmag"]), column(&header, &["VTmag"]));
    vt?;
    if (m_ra.is_none() || m_dec.is_none()) && (ra.is_none() || dec.is_none()) {
        return None;
    }
    let mut rows = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < header.len() {
            return None;
        }
        let Some(vt) = number(&fields, vt) else {
            continue;
        };
        let observed = (number(&fields, ra), number(&fields, dec));
        let (ra_deg, dec_deg) = match (number(&fields, m_ra), number(&fields, m_dec)) {
            (Some(ra), Some(dec)) => (ra, dec),
            _ => match observed {
                (Some(ra), Some(dec)) => (ra, dec),
                _ => continue,
            },
        };
        rows.push(BandRow {
            star: ReferenceStar {
                ra_deg,
                dec_deg,
                v_mag: tycho2_v_mag(vt, number(&fields, bt)),
            },
            band_dec: observed.1.unwrap_or(dec_deg),
        });
    }
    Some(rows)
}

/// The stars of one VizieR tab-separated answer: `#` lines skipped, a header
/// with the column names, a units line and a dashes line (both skipped
/// because they are not numbers), then one star per line. The mean position
/// (`RAmdeg`, `DEmdeg`) is used when present, the observed one (`RA(ICRS)`,
/// `DE(ICRS)`) otherwise (about one star in twenty has no mean position); a
/// star without `VTmag` is skipped; V is [`tycho2_v_mag`]. `None` when the
/// text has no header with those columns or ends in the middle of a row.
#[must_use]
pub fn parse_tycho2_vizier_tsv(text: &str) -> Option<Vec<ReferenceStar>> {
    parse_rows(text).map(|rows| rows.into_iter().map(|row| row.star).collect())
}

/// What a band was asked for is the observed declination; a star whose
/// position fell on the edge of two bands is kept by the lower one only, so a
/// boundary never doubles a star.
fn in_band(dec: f64, low: i32, high: i32, last: bool) -> bool {
    dec >= f64::from(low) && (dec < f64::from(high) || (last && dec <= f64::from(high)))
}

/// How many times one band is asked for from one installation before that
/// installation is given up (VizieR installations sometimes answer a busy
/// page with a 200 and no table).
const BAND_ATTEMPTS: usize = 3;

/// The pause between two attempts at the same band.
pub const BAND_RETRY_PAUSE: std::time::Duration = std::time::Duration::from_secs(3);

/// Fetches Tycho-2 band by band through `fetch` (a URL to its text), all from
/// the one installation at `base_url`, calling `progress(done, total)` after
/// each band, and returns the list. A band is asked for up to
/// [`BAND_ATTEMPTS`] times, `retry_pause` apart; one that still fails stops
/// the whole run: a list missing a strip of the sky would look complete.
pub fn download_tycho2_with(
    _consent: Tycho2Consent,
    base_url: &str,
    faintest: f64,
    retry_pause: std::time::Duration,
    fetch: &mut dyn FnMut(&str) -> Result<String, String>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<ReferenceStarList, ReferenceDownloadError> {
    let bands = declination_bands();
    let total = bands.len();
    let mut stars = Vec::new();
    for (index, (low, high)) in bands.iter().copied().enumerate() {
        let url = band_url(base_url, low, high, faintest);
        let mut attempt = 1;
        let band = loop {
            let result = fetch(&url)
                .map_err(ReferenceDownloadError::Network)
                .and_then(|text| {
                    parse_rows(&text).ok_or_else(|| {
                        ReferenceDownloadError::UnexpectedAnswer(format!("{low}..{high}"))
                    })
                });
            match result {
                Err(_) if attempt < BAND_ATTEMPTS => {
                    attempt += 1;
                    std::thread::sleep(retry_pause);
                }
                other => break other?,
            }
        };
        let last = index + 1 == total;
        stars.extend(
            band.into_iter()
                .filter(|row| row.star.v_mag <= faintest && in_band(row.band_dec, low, high, last))
                .map(|row| row.star),
        );
        progress(index + 1, total);
    }
    if stars.is_empty() {
        return Err(ReferenceDownloadError::Empty);
    }
    Ok(ReferenceStarList::new(stars))
}

/// Where a downloaded list came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceSource {
    /// The project's download host.
    Mirror,
    /// VizieR, at the CDS.
    Vizier,
    /// Another official installation of VizieR ([`VIZIER_MIRRORS`]).
    VizierMirror,
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The list read from the mirror's file, or `None` when the file is not the
/// expected one (size, digest, layout): the caller then goes to VizieR.
fn list_from_mirror(
    mirror: &Tycho2Mirror,
    fetch: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Option<ReferenceStarList> {
    let bytes = fetch(mirror.url).ok()?;
    if bytes.len() as u64 != mirror.bytes || !hex_sha256(&bytes).eq_ignore_ascii_case(mirror.sha256)
    {
        return None;
    }
    ReferenceStarList::from_bytes(&bytes)
        .ok()
        .filter(|list| !list.is_empty())
}

/// Whether a finished list is the expected one: with `expected`, its number
/// of stars and the SHA-256 of its compact bytes must both match, with no
/// tolerance.
fn is_expected(list: &ReferenceStarList, expected: Option<&Tycho2Expected>) -> bool {
    expected.is_none_or(|expected| {
        list.len() == expected.stars
            && hex_sha256(&list.to_bytes()).eq_ignore_ascii_case(expected.sha256)
    })
}

/// [`download_tycho2_with`] over several sources, in order: the `mirror`
/// first (only for the default depth, since the file holds
/// [`DEFAULT_FAINTEST`]: fetched through `fetch_mirror`, size and SHA-256
/// checked), then each of `bases` (VizieR installations). A source that
/// fails in any way (no answer, an error, a page that is not the table, a
/// table cut short, a list that is not the `expected` one at the default
/// depth) is dropped without a message to the person, who only agreed to the
/// download, and the next one starts again from the first band: bands of two
/// installations are never mixed. When every source failed, the last error is
/// returned.
#[allow(clippy::too_many_arguments)]
pub fn download_tycho2_preferring_mirror(
    consent: Tycho2Consent,
    mirror: Option<&Tycho2Mirror>,
    fetch_mirror: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
    bases: &[&str],
    expected: Option<&Tycho2Expected>,
    faintest: f64,
    retry_pause: std::time::Duration,
    fetch: &mut dyn FnMut(&str) -> Result<String, String>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(ReferenceStarList, ReferenceSource), ReferenceDownloadError> {
    let default_depth = (faintest - DEFAULT_FAINTEST).abs() < f64::EPSILON;
    if let Some(mirror) = mirror
        && default_depth
        && let Some(list) = list_from_mirror(mirror, fetch_mirror)
    {
        progress(1, 1);
        return Ok((list, ReferenceSource::Mirror));
    }
    let check = if default_depth { expected } else { None };
    let mut last_error = ReferenceDownloadError::Empty;
    for (index, base) in bases.iter().enumerate() {
        match download_tycho2_with(consent, base, faintest, retry_pause, fetch, progress) {
            Ok(list) if is_expected(&list, check) => {
                let source = if index == 0 {
                    ReferenceSource::Vizier
                } else {
                    ReferenceSource::VizierMirror
                };
                return Ok((list, source));
            }
            Ok(_) => {
                last_error = ReferenceDownloadError::NotExpected((*base).to_string());
            }
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

/// The VizieR installations in the order they are tried: the CDS, then
/// [`VIZIER_MIRRORS`].
#[must_use]
pub fn vizier_bases() -> Vec<&'static str> {
    std::iter::once(TYCHO2.vizier_base_url)
        .chain(VIZIER_MIRRORS)
        .collect()
}

/// [`download_tycho2_with`] over HTTPS, one request per band, five minutes
/// each at most.
pub fn download_tycho2(
    consent: Tycho2Consent,
    faintest: f64,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(ReferenceStarList, ReferenceSource), ReferenceDownloadError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|error| ReferenceDownloadError::Network(error.to_string()))?;
    let mut fetch = |url: &str| -> Result<String, String> {
        client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .and_then(reqwest::blocking::Response::text)
            .map_err(|error| error.to_string())
    };
    let mut fetch_mirror = |url: &str| -> Result<Vec<u8>, String> {
        client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .and_then(reqwest::blocking::Response::bytes)
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    };
    download_tycho2_preferring_mirror(
        consent,
        TYCHO2_MIRROR.as_ref(),
        &mut fetch_mirror,
        &vizier_bases(),
        TYCHO2_EXPECTED.as_ref(),
        faintest,
        BAND_RETRY_PAUSE,
        &mut fetch,
        progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = "#\n#   VizieR Astronomical Server\n#Column\tmRAdeg\t(F13.9)\tMean RA\n\nmRAdeg\tmDEdeg\tBTmag\tVTmag\tRAdeg\tDEdeg\ndeg\tdeg\tmag\tmag\tdeg\tdeg\n---------\t---------\t------\t------\t---------\t---------\n  2.31754222\t  2.23186444\t12.146\t12.146\t2.3175049\t2.2318434\n\t\t10.000\t9.000\t10.5\t-12.25\n 11.0\t -3.0\t\t\t11.0\t-3.0\n";

    #[test]
    fn the_identity_names_tycho_2_and_its_source() {
        assert_eq!(TYCHO2.id, "tycho-2");
        assert_eq!(TYCHO2.vizier_table, "I/259/tyc2");
        assert!(TYCHO2.credit.contains("Tycho-2"));
        assert!(TYCHO2.credit.contains("I/259"));
        assert!(TYCHO2.credit.contains("Credit: ESA"));
        assert!(TYCHO2.licence.contains("CC BY-NC 3.0 IGO"));
        assert_eq!(TYCHO2.list_megabytes, 25);
    }

    #[test]
    fn a_vizier_answer_is_read_with_the_catalogues_own_v_conversion() {
        let stars = parse_tycho2_vizier_tsv(ANSWER).expect("a table");
        assert_eq!(stars.len(), 2, "the star without VTmag is skipped");
        assert!(
            (stars[0].ra_deg - 2.31754222).abs() < 1e-9,
            "the mean position"
        );
        assert!((stars[0].v_mag - 12.146).abs() < 1e-9, "BT = VT: V = VT");
        // No mean position: the observed one. V = 9 - 0.09 (10 - 9).
        assert!((stars[1].ra_deg - 10.5).abs() < 1e-9);
        assert!((stars[1].dec_deg + 12.25).abs() < 1e-9);
        assert!((stars[1].v_mag - 8.91).abs() < 1e-9);
    }

    #[test]
    fn a_page_that_is_not_the_table_is_refused() {
        assert_eq!(parse_tycho2_vizier_tsv("<html>busy</html>"), None);
        assert_eq!(parse_tycho2_vizier_tsv("#only comments\n"), None);
    }

    #[test]
    fn the_bands_cover_the_whole_sky_once() {
        let bands = declination_bands();
        assert_eq!(bands.len(), 36);
        assert_eq!(bands.first(), Some(&(-90, -85)));
        assert_eq!(bands.last(), Some(&(85, 90)));
        assert!(bands.windows(2).all(|pair| pair[0].1 == pair[1].0));
    }

    #[test]
    fn a_band_query_names_the_table_the_columns_and_the_limits() {
        let url = band_url(TYCHO2.vizier_base_url, -5, 0, 12.5);
        assert!(url.starts_with("https://vizier.cds.unistra.fr/viz-bin/asu-tsv?"));
        for part in [
            "-source=I/259/tyc2",
            "-out.max=unlimited",
            "-out=RAmdeg",
            "-out=DEmdeg",
            "-out=VTmag",
            "-out=RA(ICRS)",
            "-out=DE(ICRS)",
            "VTmag=%3C12.5",
            "DE(ICRS)=-5..0",
        ] {
            assert!(url.contains(part), "{part}");
        }
    }

    #[test]
    fn the_download_runs_band_by_band_and_a_boundary_star_is_kept_once() {
        let mut asked = Vec::new();
        let mut steps = Vec::new();
        let mut fetch = |url: &str| -> Result<String, String> {
            asked.push(url.to_string());
            // Every band answers the same star on the -85 edge, which only
            // the band starting there keeps, and one inside its own band.
            Ok("mRAdeg\tmDEdeg\tBTmag\tVTmag\tRAdeg\tDEdeg\n1.0\t-85.0\t9\t9\t1.0\t-85.0\n2.0\t-87.5\t9\t9\t2.0\t-87.5\n".to_string())
        };
        let list = download_tycho2_with(
            Tycho2Consent::given(),
            TYCHO2.vizier_base_url,
            12.5,
            std::time::Duration::ZERO,
            &mut fetch,
            &mut |done, total| steps.push((done, total)),
        )
        .expect("a list");
        assert_eq!(asked.len(), 36);
        assert_eq!(steps.last(), Some(&(36, 36)));
        // -87.5 in its band once; -85.0 in the band that starts at -85 once;
        // the same two stars answered by the 34 other bands are out of band.
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn a_failing_band_stops_the_run_and_nothing_is_kept() {
        let mut calls = 0;
        let mut fetch = |_: &str| -> Result<String, String> {
            calls += 1;
            if calls >= 3 {
                Err("timed out".to_string())
            } else {
                Ok("mRAdeg\tmDEdeg\tBTmag\tVTmag\tRAdeg\tDEdeg\n".to_string())
            }
        };
        let result = download_tycho2_with(
            Tycho2Consent::given(),
            TYCHO2.vizier_base_url,
            12.5,
            std::time::Duration::ZERO,
            &mut fetch,
            &mut |_, _| {},
        );
        assert_eq!(
            result.err(),
            Some(ReferenceDownloadError::Network("timed out".to_string()))
        );
    }

    fn mirror_file() -> (Vec<u8>, Tycho2Mirror) {
        let list = ReferenceStarList::new(vec![
            ReferenceStar {
                ra_deg: 10.0,
                dec_deg: -20.0,
                v_mag: 8.0,
            },
            ReferenceStar {
                ra_deg: 11.0,
                dec_deg: 30.0,
                v_mag: 9.5,
            },
        ]);
        let bytes = list.to_bytes();
        // The digest is leaked once per test: the mirror holds `&'static str`.
        let sha256: &'static str = Box::leak(hex_sha256(&bytes).into_boxed_str());
        let mirror = Tycho2Mirror {
            url: "https://example.invalid/tycho-2.bin",
            sha256,
            bytes: bytes.len() as u64,
        };
        (bytes, mirror)
    }

    /// A VizieR answer that is one star per band, to see which source ran.
    fn vizier_answer(_: &str) -> Result<String, String> {
        Ok("mRAdeg\tmDEdeg\tBTmag\tVTmag\tRAdeg\tDEdeg\n".to_string())
    }

    #[test]
    fn no_published_file_means_vizier_alone_as_before() {
        assert_eq!(TYCHO2_MIRROR, None, "nothing is published yet");
        let mut asked_mirror = 0;
        let mut asked_vizier = 0;
        let result = download_tycho2_preferring_mirror(
            Tycho2Consent::given(),
            TYCHO2_MIRROR.as_ref(),
            &mut |_| {
                asked_mirror += 1;
                Err("never".to_string())
            },
            &[TYCHO2.vizier_base_url],
            None,
            DEFAULT_FAINTEST,
            std::time::Duration::ZERO,
            &mut |url| {
                asked_vizier += 1;
                vizier_answer(url)
            },
            &mut |_, _| {},
        );
        assert_eq!(asked_mirror, 0, "no request to a host with no file");
        assert_eq!(asked_vizier, 36, "the 36 bands of VizieR, as always");
        // These bands answer no star at all: the run ends as it always did.
        assert_eq!(result.err(), Some(ReferenceDownloadError::Empty));
    }

    #[test]
    fn the_mirror_file_is_taken_first_when_its_digest_is_right() {
        let (bytes, mirror) = mirror_file();
        let mut vizier_asked = false;
        let (list, source) = download_tycho2_preferring_mirror(
            Tycho2Consent::given(),
            Some(&mirror),
            &mut |url| {
                assert_eq!(url, mirror.url);
                Ok(bytes.clone())
            },
            &[TYCHO2.vizier_base_url],
            None,
            DEFAULT_FAINTEST,
            std::time::Duration::ZERO,
            &mut |url| {
                vizier_asked = true;
                vizier_answer(url)
            },
            &mut |_, _| {},
        )
        .expect("a list");
        assert_eq!(source, ReferenceSource::Mirror);
        assert_eq!(list.len(), 2);
        assert_eq!(list.to_bytes(), bytes);
        assert!(!vizier_asked);
    }

    #[test]
    fn a_wrong_digest_a_wrong_size_or_no_answer_falls_back_to_vizier() {
        let (bytes, mirror) = mirror_file();
        let mut other = bytes.clone();
        *other.last_mut().expect("bytes") ^= 1;
        let answers: [Result<Vec<u8>, String>; 4] = [
            Ok(other),
            Ok(bytes[..bytes.len() - 1].to_vec()),
            Err("connection refused".to_string()),
            Err("HTTP 404".to_string()),
        ];
        for answer in answers {
            let mut bands = 0;
            let result = download_tycho2_preferring_mirror(
                Tycho2Consent::given(),
                Some(&mirror),
                &mut |_| answer.clone(),
                &[TYCHO2.vizier_base_url],
                None,
                DEFAULT_FAINTEST,
                std::time::Duration::ZERO,
                &mut |url| {
                    bands += 1;
                    vizier_answer(url)
                },
                &mut |_, _| {},
            );
            assert_eq!(bands, 36, "VizieR was asked after {answer:?}");
            assert_eq!(result.err(), Some(ReferenceDownloadError::Empty));
        }
    }

    #[test]
    fn another_depth_than_the_default_never_uses_the_mirror() {
        let (bytes, mirror) = mirror_file();
        let mut asked_mirror = false;
        let _ = download_tycho2_preferring_mirror(
            Tycho2Consent::given(),
            Some(&mirror),
            &mut |_| {
                asked_mirror = true;
                Ok(bytes.clone())
            },
            &[TYCHO2.vizier_base_url],
            None,
            10.0,
            std::time::Duration::ZERO,
            &mut vizier_answer,
            &mut |_, _| {},
        );
        assert!(!asked_mirror);
    }

    // The schema VizieR serves today: `RAmdeg`, `DEmdeg`, `RA(ICRS)`, `DE(ICRS)`.
    const TODAY: &str = "RAmdeg\tDEmdeg\tBTmag\tVTmag\tRA(ICRS)\tDE(ICRS)\ndeg\tdeg\tmag\tmag\tdeg\tdeg\n------------\t------------\t------\t------\t------------\t------------\n  16.61031513\t   6.26859952\t11.468\t10.692\t 16.61052056\t  6.26860000\n\t\t12.328\t12.328\t 20.5\t  7.25\n";

    #[test]
    fn todays_column_names_are_read_and_a_star_without_a_mean_position_is_kept() {
        let stars = parse_tycho2_vizier_tsv(TODAY).expect("a table");
        assert_eq!(stars.len(), 2);
        assert!((stars[0].ra_deg - 16.61031513).abs() < 1e-9, "the mean one");
        // No mean position (a star flagged X): the observed one.
        assert!((stars[1].ra_deg - 20.5).abs() < 1e-9);
        assert!((stars[1].dec_deg - 7.25).abs() < 1e-9);
    }

    #[test]
    fn a_table_cut_short_is_refused() {
        let cut_row = TODAY.trim_end().rsplit_once('\t').expect("a tab").0;
        assert_eq!(parse_tycho2_vizier_tsv(cut_row), None, "no final newline");
        let missing_field = format!("{}\n", cut_row);
        assert_eq!(parse_tycho2_vizier_tsv(&missing_field), None, "short row");
    }

    #[test]
    fn a_band_is_decided_by_the_observed_declination() {
        // Mean declination -84.9999 (band -90..-85 by the mean), observed
        // -85.0001 (that band by the observed one), and the other way round.
        let answer = "RAmdeg\tDEmdeg\tBTmag\tVTmag\tRA(ICRS)\tDE(ICRS)\n1.0\t-84.9999\t9\t9\t1.0\t-85.0001\n2.0\t-85.0001\t9\t9\t2.0\t-84.9999\n";
        let mut fetch = |_: &str| -> Result<String, String> { Ok(answer.to_string()) };
        let list = download_tycho2_with(
            Tycho2Consent::given(),
            "https://example.invalid/asu-tsv",
            12.5,
            std::time::Duration::ZERO,
            &mut fetch,
            &mut |_, _| {},
        )
        .expect("a list");
        // Each star is kept once, by the band of its observed declination.
        assert_eq!(list.len(), 2);
    }

    const CDS: &str = "https://cds.example.invalid/asu-tsv";
    const MIRROR_A: &str = "https://a.example.invalid/asu-tsv";
    const MIRROR_B: &str = "http://b.example.invalid/asu-tsv";

    /// A table with two stars, all in the band -5..0, whatever the source.
    fn good_table(url: &str) -> String {
        if url.contains("DE(ICRS)=-5..0") {
            "RAmdeg\tDEmdeg\tBTmag\tVTmag\tRA(ICRS)\tDE(ICRS)\n10.0\t-1.0\t9\t9\t10.0\t-1.0\n11.0\t-2.0\t9.5\t9.5\t11.0\t-2.0\n".to_string()
        } else {
            "RAmdeg\tDEmdeg\tBTmag\tVTmag\tRA(ICRS)\tDE(ICRS)\n".to_string()
        }
    }

    fn expected_two_stars() -> Tycho2Expected {
        let list = ReferenceStarList::new(vec![
            ReferenceStar {
                ra_deg: 10.0,
                dec_deg: -1.0,
                v_mag: 9.0,
            },
            ReferenceStar {
                ra_deg: 11.0,
                dec_deg: -2.0,
                v_mag: 9.5,
            },
        ]);
        let sha256: &'static str = Box::leak(hex_sha256(&list.to_bytes()).into_boxed_str());
        Tycho2Expected { stars: 2, sha256 }
    }

    /// Runs the chain over `CDS`, `MIRROR_A`, `MIRROR_B` with a fake network
    /// where `answer(host, url)` is each installation's reply, and returns
    /// the result with every URL asked, in order. No real request is made.
    fn run_chain(
        expected: Option<&Tycho2Expected>,
        answer: &dyn Fn(&str, &str) -> Result<String, String>,
    ) -> (
        Result<(ReferenceStarList, ReferenceSource), ReferenceDownloadError>,
        Vec<String>,
    ) {
        let mut asked: Vec<String> = Vec::new();
        let result = download_tycho2_preferring_mirror(
            Tycho2Consent::given(),
            None,
            &mut |_| panic!("no copy of ours is published in this test"),
            &[CDS, MIRROR_A, MIRROR_B],
            expected,
            DEFAULT_FAINTEST,
            std::time::Duration::ZERO,
            &mut |url| {
                asked.push(url.to_string());
                let host = [CDS, MIRROR_A, MIRROR_B]
                    .into_iter()
                    .find(|base| url.starts_with(base))
                    .expect("a known host");
                answer(host, url)
            },
            &mut |_, _| {},
        );
        (result, asked)
    }

    #[test]
    fn the_cds_answering_is_the_only_source_asked() {
        let expected = expected_two_stars();
        let (result, asked) = run_chain(Some(&expected), &|_, url| Ok(good_table(url)));
        assert_eq!(result.expect("a list").1, ReferenceSource::Vizier);
        assert_eq!(asked.len(), 36);
        assert!(asked.iter().all(|url| url.starts_with(CDS)));
    }

    #[test]
    fn a_404_page_at_the_cds_falls_to_the_next_installation() {
        let expected = expected_two_stars();
        let (result, asked) = run_chain(Some(&expected), &|host, url| {
            if host == CDS {
                Err("HTTP status client error (404 Not Found)".to_string())
            } else {
                Ok(good_table(url))
            }
        });
        let (list, source) = result.expect("a list");
        assert_eq!(source, ReferenceSource::VizierMirror);
        assert_eq!(list.len(), 2);
        assert_eq!(
            asked.iter().filter(|u| u.starts_with(CDS)).count(),
            BAND_ATTEMPTS,
            "the first band, tried the allowed number of times"
        );
        assert_eq!(asked.iter().filter(|u| u.starts_with(MIRROR_A)).count(), 36);
        assert!(!asked.iter().any(|u| u.starts_with(MIRROR_B)));
    }

    #[test]
    fn an_html_page_with_a_200_is_a_failure_too() {
        let expected = expected_two_stars();
        let (result, _) = run_chain(Some(&expected), &|host, url| {
            if host == CDS {
                Ok("<html><body>Not found</body></html>\n".to_string())
            } else {
                Ok(good_table(url))
            }
        });
        assert_eq!(result.expect("a list").1, ReferenceSource::VizierMirror);
    }

    #[test]
    fn a_failure_midway_restarts_on_the_next_installation_and_never_mixes_bands() {
        let expected = expected_two_stars();
        let mut seen = 0;
        let seen = std::cell::RefCell::new(&mut seen);
        let (result, asked) = run_chain(Some(&expected), &|host, url| {
            if host == CDS {
                **seen.borrow_mut() += 1;
                if **seen.borrow() >= 4 {
                    return Err("connection reset".to_string());
                }
            }
            Ok(good_table(url))
        });
        assert_eq!(result.expect("a list").1, ReferenceSource::VizierMirror);
        let cds = asked.iter().filter(|u| u.starts_with(CDS)).count();
        let mirror = asked.iter().filter(|u| u.starts_with(MIRROR_A)).count();
        assert_eq!(
            (cds, mirror),
            (3 + BAND_ATTEMPTS, 36),
            "all 36 bands from one installation"
        );
        // The first request of the mirror is the first band.
        let first = asked
            .iter()
            .find(|u| u.starts_with(MIRROR_A))
            .expect("a request");
        assert!(first.contains("DE(ICRS)=-90..-85"));
    }

    #[test]
    fn a_busy_answer_is_asked_again_before_the_installation_is_given_up() {
        let expected = expected_two_stars();
        let busy_once = std::cell::Cell::new(true);
        let (result, asked) = run_chain(Some(&expected), &|_, url| {
            if url.contains("DE(ICRS)=-5..0") && busy_once.replace(false) {
                // What an installation says when its database is not there.
                return Ok("#Report from: stdb_conn_dbms, Status: STDB_ERR\n#No server is available for connection\n".to_string());
            }
            Ok(good_table(url))
        });
        assert_eq!(result.expect("a list").1, ReferenceSource::Vizier);
        assert_eq!(asked.len(), 37, "36 bands and one more attempt");
        assert!(asked.iter().all(|url| url.starts_with(CDS)));
    }

    #[test]
    fn a_table_with_fewer_stars_than_expected_is_a_failure() {
        let expected = expected_two_stars();
        let (result, asked) = run_chain(Some(&expected), &|host, url| {
            if host == CDS {
                // Valid but one star short: only the first star is answered.
                Ok(good_table(url).replace("11.0\t-2.0\t9.5\t9.5\t11.0\t-2.0\n", ""))
            } else {
                Ok(good_table(url))
            }
        });
        assert_eq!(result.expect("a list").1, ReferenceSource::VizierMirror);
        assert_eq!(asked.iter().filter(|u| u.starts_with(CDS)).count(), 36);
    }

    #[test]
    fn the_right_count_with_other_content_is_a_failure() {
        let expected = expected_two_stars();
        // Same number of stars, one magnitude different: another digest.
        let (result, _) = run_chain(Some(&expected), &|host, url| {
            if host == CDS || host == MIRROR_A {
                Ok(good_table(url).replace("9.5\t9.5", "9.6\t9.6"))
            } else {
                Ok(good_table(url))
            }
        });
        assert_eq!(result.expect("a list").1, ReferenceSource::VizierMirror);
    }

    #[test]
    fn when_every_source_fails_the_last_error_is_the_message() {
        let expected = expected_two_stars();
        let (result, asked) = run_chain(Some(&expected), &|host, _| {
            Err(format!("no answer from {host}"))
        });
        assert_eq!(
            result.err(),
            Some(ReferenceDownloadError::Network(format!(
                "no answer from {MIRROR_B}"
            )))
        );
        assert_eq!(
            asked.len(),
            3 * BAND_ATTEMPTS,
            "the first band only, per installation, then stop"
        );
    }

    #[test]
    fn the_expected_list_is_recorded_as_a_digest_and_a_count() {
        let expected = TYCHO2_EXPECTED.expect("recorded");
        assert_eq!(expected.stars, 2_411_301);
        assert_eq!(expected.sha256.len(), 64);
        assert!(
            expected
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }

    #[test]
    fn the_published_installations_are_the_official_ones_and_the_cds_comes_first() {
        let bases = vizier_bases();
        assert_eq!(bases.first(), Some(&TYCHO2.vizier_base_url));
        assert_eq!(bases.len(), 5);
        assert!(bases.iter().all(|base| base.ends_with("/viz-bin/asu-tsv")));
        // The ones that answer in HTTPS come before those that do not.
        let first_plain = bases.iter().position(|b| b.starts_with("http://"));
        assert!(
            bases[..first_plain.unwrap_or(bases.len())]
                .iter()
                .all(|b| b.starts_with("https://"))
        );
    }
}
