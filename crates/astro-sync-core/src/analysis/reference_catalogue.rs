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
//! Licence: the CDS gives the catalogue's data for use with an
//! acknowledgement; it is neither non-commercial nor share-alike. Both
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
    credit: "Tycho-2 Catalogue (Høg et al. 2000, A&A 355, L27; ESA), via the CDS, VizieR I/259",
    licence: "free use with acknowledgement (CDS, ESA); no share-alike, no non-commercial clause",
    list_megabytes: 25,
};

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

/// The VizieR query of one band: the columns `parse_tycho2_vizier_tsv`
/// reads, stars brighter than `faintest` in `VTmag`, the observed
/// declination inside the band.
#[must_use]
pub fn band_url(base: &str, low: i32, high: i32, faintest: f64) -> String {
    format!(
        "{base}?-source={}&-out.max=unlimited&-out=mRAdeg&-out=mDEdeg&-out=BTmag&-out=VTmag&-out=RAdeg&-out=DEdeg&VTmag=%3C{faintest}&DEdeg={low}..{high}",
        TYCHO2.vizier_table
    )
}

fn column(header: &[&str], name: &str) -> Option<usize> {
    header.iter().position(|candidate| candidate.trim() == name)
}

fn number(fields: &[&str], index: Option<usize>) -> Option<f64> {
    fields.get(index?)?.trim().parse::<f64>().ok()
}

/// The stars of one VizieR tab-separated answer: `#` lines skipped, a header
/// with the column names, a units line and a dashes line (both skipped
/// because they are not numbers), then one star per line. The mean position
/// (`mRAdeg`, `mDEdeg`) is used when present, the observed one (`RAdeg`,
/// `DEdeg`) otherwise; a star without `VTmag` is skipped; V is
/// [`tycho2_v_mag`]. `None` when the text has no header with those columns.
#[must_use]
pub fn parse_tycho2_vizier_tsv(text: &str) -> Option<Vec<ReferenceStar>> {
    let mut lines = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty());
    let header: Vec<&str> = lines.next()?.split('\t').collect();
    let (m_ra, m_dec) = (column(&header, "mRAdeg"), column(&header, "mDEdeg"));
    let (ra, dec) = (column(&header, "RAdeg"), column(&header, "DEdeg"));
    let (bt, vt) = (column(&header, "BTmag"), column(&header, "VTmag"));
    vt?;
    if (m_ra.is_none() || m_dec.is_none()) && (ra.is_none() || dec.is_none()) {
        return None;
    }
    let mut stars = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        let Some(vt) = number(&fields, vt) else {
            continue;
        };
        let (ra_deg, dec_deg) = match (number(&fields, m_ra), number(&fields, m_dec)) {
            (Some(ra), Some(dec)) => (ra, dec),
            _ => match (number(&fields, ra), number(&fields, dec)) {
                (Some(ra), Some(dec)) => (ra, dec),
                _ => continue,
            },
        };
        stars.push(ReferenceStar {
            ra_deg,
            dec_deg,
            v_mag: tycho2_v_mag(vt, number(&fields, bt)),
        });
    }
    Some(stars)
}

/// The observed declination of a star is what the band was asked for; a star
/// whose position fell on the edge of two bands is kept by the lower one
/// only, so a boundary never doubles a star.
fn in_band(star: &ReferenceStar, low: i32, high: i32, last: bool) -> bool {
    let dec = star.dec_deg;
    dec >= f64::from(low) && (dec < f64::from(high) || (last && dec <= f64::from(high)))
}

/// Fetches Tycho-2 band by band through `fetch` (a URL to its text), calling
/// `progress(done, total)` after each band, and returns the list. A band that
/// fails stops the whole run: a list missing a strip of the sky would look
/// complete.
pub fn download_tycho2_with(
    _consent: Tycho2Consent,
    base_url: &str,
    faintest: f64,
    fetch: &mut dyn FnMut(&str) -> Result<String, String>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<ReferenceStarList, ReferenceDownloadError> {
    let bands = declination_bands();
    let total = bands.len();
    let mut stars = Vec::new();
    for (index, (low, high)) in bands.iter().copied().enumerate() {
        let url = band_url(base_url, low, high, faintest);
        let text = fetch(&url).map_err(ReferenceDownloadError::Network)?;
        let band = parse_tycho2_vizier_tsv(&text)
            .ok_or_else(|| ReferenceDownloadError::UnexpectedAnswer(format!("{low}..{high}")))?;
        let last = index + 1 == total;
        stars.extend(
            band.into_iter()
                .filter(|star| star.v_mag <= faintest && in_band(star, low, high, last)),
        );
        progress(index + 1, total);
    }
    if stars.is_empty() {
        return Err(ReferenceDownloadError::Empty);
    }
    Ok(ReferenceStarList::new(stars))
}

/// [`download_tycho2_with`] over HTTPS, one request per band, five minutes
/// each at most.
pub fn download_tycho2(
    consent: Tycho2Consent,
    faintest: f64,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<ReferenceStarList, ReferenceDownloadError> {
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
    download_tycho2_with(
        consent,
        TYCHO2.vizier_base_url,
        faintest,
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
        for forbidden in ["non-commercial licence", "ODbL", "share-alike licence"] {
            assert!(!TYCHO2.licence.contains(forbidden));
        }
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
            "-out=VTmag",
            "VTmag=%3C12.5",
            "DEdeg=-5..0",
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
            if calls == 3 {
                Err("timed out".to_string())
            } else {
                Ok("mRAdeg\tmDEdeg\tBTmag\tVTmag\tRAdeg\tDEdeg\n".to_string())
            }
        };
        let result = download_tycho2_with(
            Tycho2Consent::given(),
            TYCHO2.vizier_base_url,
            12.5,
            &mut fetch,
            &mut |_, _| {},
        );
        assert_eq!(
            result.err(),
            Some(ReferenceDownloadError::Network("timed out".to_string()))
        );
    }
}
