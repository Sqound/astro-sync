//! The first metadata of a camera RAW file, read without a dependency and
//! without a pixel.
//!
//! A DSLR or mirrorless camera writes its RAW file (`cr2`, `cr3`, `nef`,
//! `arw`, `dng`, `raf`, `pef`, `orf`, `rw2`) as a TIFF stream (or a TIFF
//! stream in a wrapper) whose EXIF block says what the detection needs: the
//! exposure, the ISO, the date and time, the camera model, and, on a Canon,
//! the temperature of the body. They are read here the way [`crate::xisf`]
//! reads an XISF header: a handful of small reads at the offsets the file
//! names, never the sensor data, and the result is a
//! [`FitsHeader`](crate::analysis::fits::FitsHeader) of FITS-style cards, so
//! the rest of Astro Sync (the detection, the families, the darks that fit,
//! the incidents) reads a RAW pose exactly like a FITS one.
//!
//! **Why not a tool.** `exiftool` is not on a stranger's computer, `rawpy` is
//! not in the stacking environment, and Siril (which does read every RAW file
//! through LibRaw) would have to convert each file to tell its exposure: the
//! detection is run at each opening of a folder, on hundreds of files. The
//! container formats are small and public; reading them takes a few hundred
//! lines and no process. Siril still does everything that needs the pixels
//! (the conversion, the debayering, the stacking, the measure of the poses).
//!
//! **The cards.**
//!
//! | card        | from                                                                    |
//! | ----------- | ----------------------------------------------------------------------- |
//! | `INSTRUME`  | the model (prefixed with the make when it does not name it)             |
//! | `EXPTIME`   | `ExposureTime`                                                          |
//! | `GAIN`      | the ISO (`ISOSpeedRatings`, else the standard output sensitivity)        |
//! | `ISOSPEED`  | the same, as Siril writes it in the files it converts                   |
//! | `FOCALLEN`  | `FocalLength`, when the camera knows it (a T-ring says 0)               |
//! | `CCD-TEMP`  | Canon's camera temperature (the only maker whose body temperature is read) |
//! | `DATE-OBS`  | `DateTimeOriginal`, brought to UTC when the file says how (see below)    |
//! | `DATE-TZ`   | `UNKNOWN` when the file does not say the time zone of the camera clock  |
//! | `IMAGETYP`  | a word of the file name (`Light_…`, `…_dark_…`), when it says one        |
//! | `BAYERPAT`  | the mosaic is Siril's to find; a Fujifilm RAF says its own (`XTRANS` or the Bayer pattern), `RGGB` otherwise |
//! | `XBINNING`, `YBINNING` | 1                                                            |
//! | `RAWFILE`   | the extension, upper case                                               |
//!
//! **The time.** A camera writes its own clock, not UTC. The date is brought
//! to UTC when the file says how: `OffsetTimeOriginal` (modern cameras), or
//! the GPS time stamp (a camera with a GPS, or a phone's geotag). Otherwise
//! the clock is kept as the camera wrote it and `DATE-TZ` says `UNKNOWN`:
//! the night windows then compare it with the wall clock of the night
//! ([`crate::detection::NightWindow::holds_local`]) rather than with UTC.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use crate::analysis::fits::{FitsCard, FitsHeader};
use crate::time_util::{format_iso8601_utc_millis, local_datetime_to_unix_seconds};

/// The extensions of the camera RAW files Astro Sync reads (lower case).
pub const EXTENSIONS: [&str; 9] = [
    "cr2", "cr3", "nef", "arw", "dng", "raf", "pef", "orf", "rw2",
];

/// The card that marks a header as the header of a RAW file (its value is
/// the extension, upper case).
pub const RAW_FILE_CARD: &str = "RAWFILE";
/// The card that says the camera clock has no known time zone.
pub const TIME_ZONE_CARD: &str = "DATE-TZ";
/// The value of [`TIME_ZONE_CARD`] when the zone is unknown.
pub const TIME_ZONE_UNKNOWN: &str = "UNKNOWN";

/// Whether `extension` (case-insensitive, without the dot) is a RAW file.
#[must_use]
pub fn is_raw_extension(extension: &str) -> bool {
    EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension))
}

/// A tag value larger than this is not read (a maker note of a few kilobytes
/// is the largest thing wanted).
const MAX_VALUE_BYTES: u64 = 256 * 1024;
/// An IFD with more entries than this is not a metadata block.
const MAX_ENTRIES: u16 = 2048;

/// What a RAW file's metadata says (every field is optional: a file says
/// what its maker wrote).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawMetadata {
    pub make: Option<String>,
    pub model: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub iso: Option<f64>,
    pub focal_length_mm: Option<f64>,
    pub temperature_c: Option<f64>,
    /// `DateTimeOriginal`, the camera clock as written.
    pub taken: Option<WallClock>,
    /// `OffsetTimeOriginal`, seconds east of UTC.
    pub offset_seconds: Option<i64>,
    /// The GPS time stamp, Unix seconds (UTC).
    pub gps_unix: Option<f64>,
    /// The mosaic, when the file says it (a Fujifilm RAF does; the other
    /// formats leave it to Siril).
    pub mosaic: Option<Mosaic>,
}

/// The colour filter array of a sensor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mosaic {
    /// A 2 x 2 Bayer pattern (`RGGB`, `GRBG`…).
    Bayer(String),
    /// Fujifilm's 6 x 6 X-Trans pattern: Siril debayers it when it converts
    /// the file, not when it calibrates it (see `stacking/raw_files.py`).
    XTrans,
}

/// A date and time as a camera clock shows them (no zone).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallClock {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// Seconds with the sub-second digits (`SubSecTimeOriginal`).
    pub second: f64,
}

impl WallClock {
    /// The wall clock read as if it were UTC, in Unix seconds.
    fn as_unix(self) -> f64 {
        local_datetime_to_unix_seconds(
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            0,
        )
    }
}

/* ------------------------------- The reader ------------------------------ */

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// Random access to the bytes of a file (or of a buffer in the tests).
struct Source<R: Read + Seek> {
    inner: R,
    length: u64,
}

impl<R: Read + Seek> Source<R> {
    fn new(mut inner: R) -> io::Result<Self> {
        let length = inner.seek(SeekFrom::End(0))?;
        Ok(Self { inner, length })
    }

    fn read_at(&mut self, offset: u64, length: u64) -> io::Result<Vec<u8>> {
        let end = offset
            .checked_add(length)
            .ok_or_else(|| invalid("offset out of range"))?;
        if end > self.length || length > MAX_VALUE_BYTES * 4 {
            return Err(invalid("read beyond the end of the file"));
        }
        self.inner.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0_u8; length as usize];
        self.inner.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Order {
    little: bool,
}

impl Order {
    fn u16(self, bytes: &[u8]) -> u16 {
        let pair = [bytes[0], bytes[1]];
        if self.little {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    }

    fn u32(self, bytes: &[u8]) -> u32 {
        let quad = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if self.little {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        }
    }
}

/// One entry of an IFD.
#[derive(Debug, Clone, Copy)]
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    /// The four bytes of the value field (the value itself when it fits).
    field: [u8; 4],
}

/// The TIFF stream that starts at `base` of the source: its byte order and
/// the offset of its first IFD.
#[derive(Debug, Clone, Copy)]
struct Tiff {
    base: u64,
    order: Order,
    first_ifd: u64,
}

fn kind_size(kind: u16) -> u64 {
    match kind {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 | 16 | 17 | 18 => 8,
        _ => 0,
    }
}

impl Tiff {
    /// The header of a TIFF stream: `II` or `MM`, a magic number (42 for a
    /// TIFF, and the ones Olympus and Panasonic put there), the offset of
    /// the first IFD.
    fn open<R: Read + Seek>(source: &mut Source<R>, base: u64) -> io::Result<Self> {
        let head = source.read_at(base, 8)?;
        let order = match &head[..2] {
            b"II" => Order { little: true },
            b"MM" => Order { little: false },
            _ => return Err(invalid("not a TIFF stream")),
        };
        // 0x2A: TIFF (Canon, Nikon, Sony, Pentax, DNG); 0x4F52 / 0x5352:
        // Olympus; 0x55: Panasonic.
        if !matches!(order.u16(&head[2..4]), 0x2A | 0x4F52 | 0x5352 | 0x55) {
            return Err(invalid("not a TIFF stream"));
        }
        Ok(Self {
            base,
            order,
            first_ifd: u64::from(order.u32(&head[4..8])),
        })
    }

    fn entries<R: Read + Seek>(
        self,
        source: &mut Source<R>,
        offset: u64,
    ) -> io::Result<Vec<Entry>> {
        let at = self.base + offset;
        let count = self.order.u16(&source.read_at(at, 2)?);
        if count == 0 || count > MAX_ENTRIES {
            return Err(invalid("not an IFD"));
        }
        let raw = source.read_at(at + 2, u64::from(count) * 12)?;
        Ok(raw
            .chunks_exact(12)
            .map(|chunk| Entry {
                tag: self.order.u16(&chunk[0..2]),
                kind: self.order.u16(&chunk[2..4]),
                count: self.order.u32(&chunk[4..8]),
                field: [chunk[8], chunk[9], chunk[10], chunk[11]],
            })
            .collect())
    }

    /// The bytes of an entry's value.
    fn value<R: Read + Seek>(self, source: &mut Source<R>, entry: Entry) -> io::Result<Vec<u8>> {
        let size = kind_size(entry.kind) * u64::from(entry.count);
        if size == 0 || size > MAX_VALUE_BYTES {
            return Err(invalid("value out of range"));
        }
        if size <= 4 {
            return Ok(entry.field[..size as usize].to_vec());
        }
        let offset = u64::from(self.order.u32(&entry.field));
        source.read_at(self.base + offset, size)
    }

    fn pointer(self, entry: Entry) -> u64 {
        u64::from(self.order.u32(&entry.field))
    }

    fn text<R: Read + Seek>(self, source: &mut Source<R>, entry: Entry) -> Option<String> {
        let bytes = self.value(source, entry).ok()?;
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        let text = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Unsigned integers of a SHORT or LONG (or BYTE) entry.
    fn integers<R: Read + Seek>(self, source: &mut Source<R>, entry: Entry) -> Vec<u32> {
        let Ok(bytes) = self.value(source, entry) else {
            return Vec::new();
        };
        match entry.kind {
            1 | 7 => bytes.iter().map(|b| u32::from(*b)).collect(),
            3 => bytes
                .chunks_exact(2)
                .map(|c| u32::from(self.order.u16(c)))
                .collect(),
            4 => bytes.chunks_exact(4).map(|c| self.order.u32(c)).collect(),
            _ => Vec::new(),
        }
    }

    /// Signed 16-bit integers of a SHORT or SSHORT entry (Canon's arrays).
    fn signed_shorts<R: Read + Seek>(self, source: &mut Source<R>, entry: Entry) -> Vec<i16> {
        let Ok(bytes) = self.value(source, entry) else {
            return Vec::new();
        };
        if !matches!(entry.kind, 3 | 8) {
            return Vec::new();
        }
        bytes
            .chunks_exact(2)
            .map(|c| self.order.u16(c) as i16)
            .collect()
    }

    /// The rationals of a RATIONAL entry, as numbers.
    fn rationals<R: Read + Seek>(self, source: &mut Source<R>, entry: Entry) -> Vec<f64> {
        if entry.kind != 5 {
            return Vec::new();
        }
        let Ok(bytes) = self.value(source, entry) else {
            return Vec::new();
        };
        bytes
            .chunks_exact(8)
            .filter_map(|c| {
                let numerator = f64::from(self.order.u32(&c[0..4]));
                let denominator = f64::from(self.order.u32(&c[4..8]));
                (denominator != 0.0).then(|| numerator / denominator)
            })
            .collect()
    }
}

fn find(entries: &[Entry], tag: u16) -> Option<Entry> {
    entries.iter().find(|entry| entry.tag == tag).copied()
}

/* ------------------------------ The collecting --------------------------- */

/// What the blocks of a file add up to.
#[derive(Debug, Default)]
struct Collected {
    meta: RawMetadata,
    /// `DateTime` of the main block, the fallback of `DateTimeOriginal`.
    date_time: Option<String>,
    date_time_original: Option<String>,
    sub_second: Option<String>,
    offset_original: Option<String>,
    gps_date: Option<String>,
    gps_time: Option<(f64, f64, f64)>,
}

impl Collected {
    /// The main block (IFD0): the maker and model, the pointers.
    fn main_block<R: Read + Seek>(
        &mut self,
        tiff: Tiff,
        source: &mut Source<R>,
        entries: &[Entry],
    ) {
        if let Some(entry) = find(entries, 0x010F) {
            self.meta.make = self.meta.make.take().or_else(|| tiff.text(source, entry));
        }
        if let Some(entry) = find(entries, 0x0110) {
            self.meta.model = self.meta.model.take().or_else(|| tiff.text(source, entry));
        }
        if let Some(entry) = find(entries, 0x0132) {
            self.date_time = self.date_time.take().or_else(|| tiff.text(source, entry));
        }
        // Panasonic keeps its ISO in the main block.
        if self.meta.iso.is_none()
            && let Some(entry) = find(entries, 0x0017)
        {
            self.meta.iso = tiff
                .integers(source, entry)
                .first()
                .map(|v| f64::from(*v))
                .filter(|v| *v > 0.0);
        }
    }

    /// The EXIF block.
    fn exif_block<R: Read + Seek>(
        &mut self,
        tiff: Tiff,
        source: &mut Source<R>,
        entries: &[Entry],
    ) {
        if let Some(entry) = find(entries, 0x829A) {
            self.meta.exposure_seconds = tiff
                .rationals(source, entry)
                .first()
                .copied()
                .filter(|v| v.is_finite() && *v > 0.0);
        }
        // The ISO: the standard output sensitivity of the recommended index
        // and the ISO speed (0x8832, 0x8833) say it when 0x8827 says 65535
        // (some cameras on automatic ISO), and are the ISO of the cameras
        // that do not write 0x8827.
        let iso_of = |tag: u16, source: &mut Source<R>| {
            find(entries, tag)
                .and_then(|entry| tiff.integers(source, entry).first().copied())
                .filter(|v| *v > 0 && *v < 65_535)
                .map(f64::from)
        };
        if let Some(iso) = iso_of(0x8827, source)
            .or_else(|| iso_of(0x8833, source))
            .or_else(|| iso_of(0x8832, source))
        {
            self.meta.iso = Some(iso);
        }
        if let Some(entry) = find(entries, 0x920A) {
            self.meta.focal_length_mm = tiff
                .rationals(source, entry)
                .first()
                .copied()
                .filter(|v| v.is_finite() && *v > 0.0);
        }
        if let Some(entry) = find(entries, 0x9003) {
            self.date_time_original = tiff.text(source, entry);
        }
        if let Some(entry) = find(entries, 0x9291) {
            self.sub_second = tiff.text(source, entry);
        }
        // `OffsetTimeOriginal`, else `OffsetTime` (the camera's own zone).
        if let Some(entry) = find(entries, 0x9011).or_else(|| find(entries, 0x9010)) {
            self.offset_original = tiff.text(source, entry);
        }
    }

    fn gps_block<R: Read + Seek>(&mut self, tiff: Tiff, source: &mut Source<R>, entries: &[Entry]) {
        if let Some(entry) = find(entries, 0x001D) {
            self.gps_date = tiff.text(source, entry);
        }
        if let Some(entry) = find(entries, 0x0007) {
            let parts = tiff.rationals(source, entry);
            if let [hours, minutes, seconds] = parts[..] {
                self.gps_time = Some((hours, minutes, seconds));
            }
        }
    }

    /// Canon's maker note: the camera temperature, in the array of the shot
    /// information (tag 4), at the position the maker documents.
    fn canon_block<R: Read + Seek>(
        &mut self,
        tiff: Tiff,
        source: &mut Source<R>,
        entries: &[Entry],
    ) {
        let Some(entry) = find(entries, 0x0004) else {
            return;
        };
        let shot = tiff.signed_shorts(source, entry);
        // Position 12: `CameraTemperature`, the temperature in degrees plus
        // 128 (0 = not recorded).
        if let Some(raw) = shot.get(12).copied()
            && raw > 0
        {
            let degrees = f64::from(raw) - 128.0;
            if (-40.0..=80.0).contains(&degrees) {
                self.meta.temperature_c = Some(degrees);
            }
        }
    }

    /// A TIFF stream with its main block, EXIF block, GPS block and maker
    /// note: the shape of a CR2, NEF, ARW, DNG, PEF, ORF, RW2 file, and of
    /// the EXIF block of a RAF's JPEG.
    fn whole_tiff<R: Read + Seek>(&mut self, tiff: Tiff, source: &mut Source<R>) -> io::Result<()> {
        let main = tiff.entries(source, tiff.first_ifd)?;
        self.main_block(tiff, source, &main);
        if let Some(pointer) = find(&main, 0x8825)
            && let Ok(gps) = tiff.entries(source, tiff.pointer(pointer))
        {
            self.gps_block(tiff, source, &gps);
        }
        if let Some(pointer) = find(&main, 0x8769)
            && let Ok(exif) = tiff.entries(source, tiff.pointer(pointer))
        {
            self.exif_block(tiff, source, &exif);
            if let Some(note) = find(&exif, 0x927C)
                && self.is_canon()
                && kind_size(note.kind) * u64::from(note.count) > 4
                && let Ok(canon) = tiff.entries(source, tiff.pointer(note))
            {
                self.canon_block(tiff, source, &canon);
            }
        }
        Ok(())
    }

    fn is_canon(&self) -> bool {
        self.meta
            .make
            .as_deref()
            .is_some_and(|make| make.to_ascii_lowercase().starts_with("canon"))
    }

    /// The moment, once every block is read.
    fn finish(mut self) -> RawMetadata {
        let text = self
            .date_time_original
            .take()
            .or_else(|| self.date_time.take());
        self.meta.taken = text.as_deref().and_then(parse_wall_clock).map(|mut clock| {
            if let Some(digits) = self.sub_second.as_deref() {
                let digits: String = digits.chars().filter(char::is_ascii_digit).collect();
                if !digits.is_empty()
                    && let Ok(fraction) = format!("0.{digits}").parse::<f64>()
                {
                    clock.second += fraction;
                }
            }
            clock
        });
        self.meta.offset_seconds = self
            .offset_original
            .as_deref()
            .and_then(|text| crate::time_util::parse_fixed_offset_seconds(text.trim()));
        if let (Some(date), Some((hours, minutes, seconds))) =
            (self.gps_date.as_deref(), self.gps_time)
            && let Some(day) = parse_wall_clock(&format!("{} 00:00:00", date.trim()))
        {
            self.meta.gps_unix = Some(day.as_unix() + hours * 3600.0 + minutes * 60.0 + seconds);
        }
        self.meta
    }
}

/// `2026:10:07 21:03:12` (or with dashes) as a wall clock; the all-zero or
/// blank stamp a camera without a clock writes is not one.
fn parse_wall_clock(text: &str) -> Option<WallClock> {
    let text = text.trim();
    let (date, time) = text.split_once([' ', 'T'])?;
    let mut date = date.split([':', '-']);
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let mut time = time.split(':');
    let hour: u32 = time.next()?.parse().ok()?;
    let minute: u32 = time.next()?.parse().ok()?;
    let second: f64 = time.next()?.trim().parse().ok()?;
    let plausible = (1990..=2100).contains(&year)
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && (0.0..61.0).contains(&second);
    plausible.then_some(WallClock {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

/* --------------------------------- Wrappers ------------------------------- */

/// A Fujifilm RAF: a 16-byte signature, then, at 84, the offset and length
/// (big endian) of the JPEG preview whose EXIF block holds the metadata.
fn read_raf<R: Read + Seek>(source: &mut Source<R>, collected: &mut Collected) -> io::Result<()> {
    let head = source.read_at(0, 100)?;
    let jpeg = u64::from(u32::from_be_bytes([head[84], head[85], head[86], head[87]]));
    let length = u64::from(u32::from_be_bytes([head[88], head[89], head[90], head[91]]));
    if jpeg == 0 || jpeg >= source.length {
        return Err(invalid("no preview in the RAF"));
    }
    if collected.meta.make.is_none() {
        collected.meta.make = Some("FUJIFILM".to_string());
    }
    // The CFA header (offset and length at 92 and 96): its record 0x131 is
    // the pattern, 36 bytes for an X-Trans, 4 for a Bayer.
    let cfa = u64::from(u32::from_be_bytes([head[92], head[93], head[94], head[95]]));
    if cfa > 0 && cfa + 8 <= source.length {
        collected.meta.mosaic = raf_mosaic(&source.read_at(cfa, 1024.min(source.length - cfa))?);
    }
    let end = (jpeg + length.max(65_536)).min(source.length);
    let mut position = jpeg;
    if source.read_at(position, 2)? != [0xFF, 0xD8] {
        return Err(invalid("the RAF preview is not a JPEG"));
    }
    position += 2;
    while position + 4 <= end {
        let marker = source.read_at(position, 4)?;
        if marker[0] != 0xFF {
            break;
        }
        let size = u64::from(u16::from_be_bytes([marker[2], marker[3]]));
        match marker[1] {
            // Start of scan: the metadata would have come before.
            0xDA | 0xD9 => break,
            0xE1 if size >= 8 => {
                let id = source.read_at(position + 4, 6)?;
                if id == *b"Exif\0\0" {
                    let tiff = Tiff::open(source, position + 10)?;
                    return collected.whole_tiff(tiff, source);
                }
            }
            _ => {}
        }
        position += 2 + size;
    }
    Err(invalid("no EXIF block in the RAF preview"))
}

/// The mosaic named by a RAF's CFA header: a count of records, each a tag, a
/// size (both 16 bits) and its bytes, all big endian.
fn raf_mosaic(header: &[u8]) -> Option<Mosaic> {
    let count = u32::from_be_bytes(header.get(..4)?.try_into().ok()?);
    let mut position = 4_usize;
    for _ in 0..count.min(32) {
        let tag = u16::from_be_bytes(header.get(position..position + 2)?.try_into().ok()?);
        let size = usize::from(u16::from_be_bytes(
            header.get(position + 2..position + 4)?.try_into().ok()?,
        ));
        let data = header.get(position + 4..position + 4 + size)?;
        if tag == 0x131 {
            return match data {
                [a, b, c, d] => {
                    let letter = |v: u8| match v {
                        0 => 'R',
                        1 => 'G',
                        _ => 'B',
                    };
                    Some(Mosaic::Bayer(
                        [a, b, c, d].map(|v| letter(*v)).iter().collect(),
                    ))
                }
                _ if size == 36 => Some(Mosaic::XTrans),
                _ => None,
            };
        }
        position += 4 + size;
    }
    None
}

/// The user type of the Canon `uuid` box that holds the CR3's metadata.
const CANON_UUID: [u8; 16] = [
    0x85, 0xC0, 0xB6, 0x87, 0x82, 0x0F, 0x11, 0xE0, 0x81, 0x11, 0xF4, 0xCE, 0x46, 0x2B, 0x6A, 0x48,
];

/// One ISO base media box: its type, where its content starts and ends.
struct BoxAt {
    kind: [u8; 4],
    content: u64,
    end: u64,
}

fn boxes_in<R: Read + Seek>(
    source: &mut Source<R>,
    start: u64,
    end: u64,
) -> io::Result<Vec<BoxAt>> {
    let mut found = Vec::new();
    let mut position = start;
    while position + 8 <= end && found.len() < 256 {
        let head = source.read_at(position, 8)?;
        let size32 = u64::from(u32::from_be_bytes([head[0], head[1], head[2], head[3]]));
        let kind = [head[4], head[5], head[6], head[7]];
        let (header, size) = match size32 {
            1 => {
                let wide = source.read_at(position + 8, 8)?;
                let mut quad = [0_u8; 8];
                quad.copy_from_slice(&wide);
                (16, u64::from_be_bytes(quad))
            }
            0 => (8, end - position),
            size => (8, size),
        };
        if size < header || position + size > end {
            break;
        }
        found.push(BoxAt {
            kind,
            content: position + header,
            end: position + size,
        });
        position += size;
    }
    Ok(found)
}

/// A Canon CR3: ISO base media boxes; `moov` holds a `uuid` box whose
/// children `CMT1` (the main block), `CMT2` (the EXIF block), `CMT3` (the
/// maker note) and `CMT4` (the GPS block) are each a small TIFF stream of
/// their own.
fn read_cr3<R: Read + Seek>(source: &mut Source<R>, collected: &mut Collected) -> io::Result<()> {
    let length = source.length;
    let top = boxes_in(source, 0, length)?;
    let moov = top
        .iter()
        .find(|b| &b.kind == b"moov")
        .ok_or_else(|| invalid("no moov box"))?;
    for child in boxes_in(source, moov.content, moov.end)? {
        if &child.kind != b"uuid" || child.end < child.content + 16 {
            continue;
        }
        if source.read_at(child.content, 16)? != CANON_UUID {
            continue;
        }
        for part in boxes_in(source, child.content + 16, child.end)? {
            let Ok(tiff) = Tiff::open(source, part.content) else {
                continue;
            };
            let Ok(entries) = tiff.entries(source, tiff.first_ifd) else {
                continue;
            };
            match &part.kind {
                b"CMT1" => collected.main_block(tiff, source, &entries),
                b"CMT2" => collected.exif_block(tiff, source, &entries),
                b"CMT3" => {
                    if collected.meta.make.is_none() {
                        collected.meta.make = Some("Canon".to_string());
                    }
                    collected.canon_block(tiff, source, &entries);
                }
                b"CMT4" => collected.gps_block(tiff, source, &entries),
                _ => {}
            }
        }
        return Ok(());
    }
    Err(invalid("no Canon metadata in the CR3"))
}

/// The metadata of the RAW file behind `source`.
fn read_metadata<R: Read + Seek>(source: &mut Source<R>) -> io::Result<RawMetadata> {
    let head = source.read_at(0, 16.min(source.length))?;
    let mut collected = Collected::default();
    if head.starts_with(b"FUJIFILMCCD-RAW") {
        read_raf(source, &mut collected)?;
    } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
        read_cr3(source, &mut collected)?;
    } else {
        let tiff = Tiff::open(source, 0)?;
        collected.whole_tiff(tiff, source)?;
    }
    Ok(collected.finish())
}

/// The metadata of the RAW file at `path` (a few small reads).
pub fn read_metadata_file(path: &Path) -> io::Result<RawMetadata> {
    let mut source = Source::new(File::open(path)?)?;
    read_metadata(&mut source)
}

/* ------------------------------- The header ------------------------------ */

/// The frame type a file name says (`Light_M31_0001.CR2`, `M31-dark-300s.nef`):
/// a whole word of the name, in the words capture programs write. `None`
/// when no word says one, or when the words disagree. A RAW file has no
/// header card for it.
#[must_use]
pub fn frame_type_of_name(file_name: &str) -> Option<&'static str> {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let mut found: Option<&'static str> = None;
    for word in stem
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let kind = match word.to_lowercase().as_str() {
            "light" | "lights" => "Light",
            "dark" | "darks" => "Dark",
            "flat" | "flats" => "Flat",
            "bias" | "biases" | "offset" | "offsets" => "Bias",
            "darkflat" | "darkflats" | "flatdark" | "flatdarks" => "DarkFlat",
            _ => continue,
        };
        match found {
            Some(previous) if previous != kind => return None,
            _ => found = Some(kind),
        }
    }
    found
}

fn card(keyword: &str, value: impl Into<String>) -> FitsCard {
    FitsCard {
        keyword: keyword.to_string(),
        value: Some(value.into()),
        comment: None,
    }
}

fn number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value}")
    }
}

/// `DATE-OBS` of the metadata (`YYYY-MM-DDTHH:MM:SS[.sss]`, no zone letter)
/// and whether the camera clock's zone is unknown.
fn date_obs(meta: &RawMetadata) -> Option<(String, bool)> {
    let clock = meta.taken?;
    let wall = clock.as_unix();
    // The GPS time says the zone of the clock when it is within a day of it
    // (rounded to the quarter hour: the shutter and the GPS fix differ by a
    // few seconds).
    let gps_offset = meta.gps_unix.and_then(|gps| {
        let seconds = (wall - gps).round();
        let quarter = (seconds / 900.0).round() * 900.0;
        ((quarter.abs()) <= 14.0 * 3600.0 && (seconds - quarter).abs() <= 120.0)
            .then_some(quarter as i64)
    });
    let (unix, unknown) = match meta.offset_seconds.or(gps_offset) {
        Some(offset) => (wall - offset as f64, false),
        None => (wall, true),
    };
    let text = format_iso8601_utc_millis(unix);
    let text = text.strip_suffix('Z').unwrap_or(&text);
    let text = text.strip_suffix(".000").unwrap_or(text);
    Some((text.to_string(), unknown))
}

/// The FITS-style cards of a RAW file (see the module documentation).
#[must_use]
pub fn header_of(meta: &RawMetadata, file_name: &str, extension: &str) -> FitsHeader {
    let mut cards = Vec::new();
    if let Some(kind) = frame_type_of_name(file_name) {
        cards.push(card("IMAGETYP", kind));
    }
    let make = meta.make.as_deref().map(str::trim).unwrap_or_default();
    if let Some(model) = meta.model.as_deref() {
        // « NIKON CORPORATION » + « NIKON D3300 »: the model names the make
        // when it holds the make's first word.
        let first_word = make.split_whitespace().next().unwrap_or_default();
        let named = model
            .to_ascii_lowercase()
            .contains(&first_word.to_ascii_lowercase());
        let name = if first_word.is_empty() || named {
            model.to_string()
        } else {
            format!("{first_word} {model}")
        };
        cards.push(card("INSTRUME", name));
    }
    if let Some(seconds) = meta.exposure_seconds {
        cards.push(card("EXPTIME", number(seconds)));
    }
    if let Some(iso) = meta.iso {
        cards.push(card("GAIN", number(iso)));
        cards.push(card("ISOSPEED", number(iso)));
    }
    if let Some(focal) = meta.focal_length_mm {
        cards.push(card("FOCALLEN", number(focal)));
    }
    if let Some(degrees) = meta.temperature_c {
        cards.push(card("CCD-TEMP", number(degrees)));
    }
    if let Some((date, unknown)) = date_obs(meta) {
        cards.push(card("DATE-OBS", date));
        if unknown {
            cards.push(card(TIME_ZONE_CARD, TIME_ZONE_UNKNOWN));
        }
    }
    cards.push(card("XBINNING", "1"));
    cards.push(card("YBINNING", "1"));
    cards.push(card(
        "BAYERPAT",
        match &meta.mosaic {
            Some(Mosaic::XTrans) => "XTRANS",
            Some(Mosaic::Bayer(pattern)) => pattern,
            None => "RGGB",
        },
    ));
    cards.push(card(RAW_FILE_CARD, extension.to_ascii_uppercase()));
    FitsHeader { cards }
}

/// The header of the RAW file at `path`; an error when the file is not a
/// RAW file this reader knows (a variant it cannot read).
pub fn read_header_file(path: &Path) -> io::Result<FitsHeader> {
    let meta = read_metadata_file(path)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    Ok(header_of(&meta, file_name, extension))
}

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;
