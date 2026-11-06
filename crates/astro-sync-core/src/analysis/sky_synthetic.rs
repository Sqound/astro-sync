//! Test-only synthetic frames for the measured sky background: a FITS light
//! frame built in memory from a known sky level, a known pedestal, known
//! stars and a known noise, so that every step of `analysis::sky`,
//! `analysis::photometry` and `analysis::sky_night` can be checked against
//! the truth it was made from. No file is committed: like `fits::tests`,
//! the format is mechanical and the builder makes it explicit.

use super::fits::{BLOCK_LEN, CARD_LEN};

/// A small, deterministic generator (SplitMix64): the same seed always gives
/// the same frame, so a tolerance that holds once holds on every run.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in (0, 1), never exactly 0.
    pub(crate) fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// Standard normal (Box-Muller).
    pub(crate) fn gaussian(&mut self) -> f64 {
        let u1 = self.uniform();
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

/// One star drawn on the frame: its centre in FITS pixel coordinates (the
/// first pixel's centre is (1, 1)) and its total flux in ADU.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SyntheticStar {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) flux_adu: f64,
    /// Its own Gaussian width (an extended object), else the frame's PSF.
    pub(crate) sigma: Option<f64>,
}

impl SyntheticStar {
    /// A star with the frame's PSF.
    pub(crate) fn point(x: f64, y: f64, flux_adu: f64) -> Self {
        Self {
            x,
            y,
            flux_adu,
            sigma: None,
        }
    }
}

/// How the pixels are stored in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Storage {
    /// `BITPIX = 16`, `BZERO = 32768`: unsigned 16-bit, the usual camera file.
    Unsigned16,
    /// `BITPIX = -32`: 32-bit floats, a calibrated or stacked file.
    Float32,
}

/// A frame: sky + pedestal + stars + noise, in ADU per sensor pixel.
#[derive(Debug, Clone)]
pub(crate) struct SyntheticFrame {
    pub(crate) width: usize,
    pub(crate) height: usize,
    /// The sky alone, per sensor pixel, over the whole exposure.
    pub(crate) sky_adu: f64,
    /// The level the sensor adds to every pixel (bias, offset).
    pub(crate) pedestal_adu: f64,
    /// Gaussian PSF, in sensor pixels.
    pub(crate) sigma_psf: f64,
    pub(crate) read_noise_adu: f64,
    /// Electrons per ADU for the shot noise.
    pub(crate) gain: f64,
    pub(crate) stars: Vec<SyntheticStar>,
    pub(crate) seed: u64,
    pub(crate) storage: Storage,
    /// The highest value the sensor records (16-bit full well).
    pub(crate) clip_adu: f64,
}

impl SyntheticFrame {
    pub(crate) fn new(width: usize, height: usize, sky_adu: f64) -> Self {
        Self {
            width,
            height,
            sky_adu,
            pedestal_adu: 0.0,
            sigma_psf: 1.6,
            read_noise_adu: 3.0,
            gain: 1.0,
            stars: Vec::new(),
            seed: 1,
            storage: Storage::Unsigned16,
            clip_adu: 65535.0,
        }
    }

    /// The noiseless and noisy pixel values, row by row (row 0 = FITS row 1).
    pub(crate) fn render(&self) -> Vec<f64> {
        let mut signal = vec![self.sky_adu; self.width * self.height];
        for star in &self.stars {
            let sigma = star.sigma.unwrap_or(self.sigma_psf);
            let reach = (6.0 * sigma).ceil() as i64 + 1;
            let cx = star.x - 1.0; // 0-based centre
            let cy = star.y - 1.0;
            let x0 = (cx.round() as i64 - reach).max(0);
            let x1 = (cx.round() as i64 + reach).min(self.width as i64 - 1);
            let y0 = (cy.round() as i64 - reach).max(0);
            let y1 = (cy.round() as i64 + reach).min(self.height as i64 - 1);
            for y in y0..=y1 {
                let fy = pixel_fraction(y as f64, cy, sigma);
                if fy == 0.0 {
                    continue;
                }
                for x in x0..=x1 {
                    let fx = pixel_fraction(x as f64, cx, sigma);
                    signal[y as usize * self.width + x as usize] += star.flux_adu * fx * fy;
                }
            }
        }
        let mut rng = Rng::new(self.seed);
        signal
            .into_iter()
            .map(|value| {
                let sigma = (value.max(0.0) / self.gain + self.read_noise_adu.powi(2)).sqrt();
                let noisy = self.pedestal_adu + value + sigma * rng.gaussian();
                match self.storage {
                    Storage::Unsigned16 => noisy.round().clamp(0.0, self.clip_adu),
                    Storage::Float32 => noisy.min(self.clip_adu),
                }
            })
            .collect()
    }

    /// The whole FITS file: the primary header with `cards` after the
    /// mandatory ones, then the data unit, both padded to whole blocks.
    pub(crate) fn to_fits(&self, cards: &[(&str, String)]) -> Vec<u8> {
        let pixels = self.render();
        let mut all_cards: Vec<(String, String)> = vec![
            ("SIMPLE".into(), "T".into()),
            (
                "BITPIX".into(),
                match self.storage {
                    Storage::Unsigned16 => "16".into(),
                    Storage::Float32 => "-32".into(),
                },
            ),
            ("NAXIS".into(), "2".into()),
            ("NAXIS1".into(), self.width.to_string()),
            ("NAXIS2".into(), self.height.to_string()),
        ];
        if self.storage == Storage::Unsigned16 {
            all_cards.push(("BZERO".into(), "32768".into()));
            all_cards.push(("BSCALE".into(), "1".into()));
        }
        for (keyword, value) in cards {
            all_cards.push(((*keyword).to_string(), value.clone()));
        }
        let mut out = header_bytes(&all_cards);
        for value in pixels {
            match self.storage {
                Storage::Unsigned16 => {
                    let stored = (value as i64 - 32768) as i16;
                    out.extend_from_slice(&stored.to_be_bytes());
                }
                Storage::Float32 => out.extend_from_slice(&(value as f32).to_be_bytes()),
            }
        }
        let remainder = out.len() % BLOCK_LEN;
        if remainder != 0 {
            out.resize(out.len() + (BLOCK_LEN - remainder), 0);
        }
        out
    }
}

/// A header made of `cards`, closed by `END` and padded with blanks.
pub(crate) fn header_bytes(cards: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (keyword, value) in cards {
        let mut bytes = format!("{keyword:<8}= {value}").into_bytes();
        bytes.resize(CARD_LEN, b' ');
        out.extend_from_slice(&bytes);
    }
    let mut end_card = b"END".to_vec();
    end_card.resize(CARD_LEN, b' ');
    out.extend_from_slice(&end_card);
    let remainder = out.len() % BLOCK_LEN;
    if remainder != 0 {
        out.resize(out.len() + (BLOCK_LEN - remainder), b' ');
    }
    out
}

/// The share of a unit Gaussian centred at `centre` that falls on the pixel
/// whose centre is `index` (0-based): the exact integral over the pixel.
fn pixel_fraction(index: f64, centre: f64, sigma: f64) -> f64 {
    let scale = sigma * std::f64::consts::SQRT_2;
    0.5 * (erf((index + 0.5 - centre) / scale) - erf((index - 0.5 - centre) / scale))
}

/// The error function (Abramowitz and Stegun 7.1.26, |error| < 1.5e-7).
pub(crate) fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    sign * (1.0 - poly * (-x * x).exp())
}

/// A quoted FITS string value.
pub(crate) fn quoted(text: &str) -> String {
    format!("'{text}'")
}
