//! Small camera RAW files for the tests of this crate: a TIFF stream
//! built entry by entry, and a file a camera could have written.

use std::path::Path;

/* ------------------------- A builder of small TIFF streams ---------------- */

/// A value of an IFD entry, as the test writes it.
#[derive(Clone)]
pub(crate) enum V {
    Ascii(&'static str),
    /// An ASCII value the test computed.
    Text(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    /// A pointer to the IFD of that rank in the stream.
    Ptr(usize),
    /// A pointer to the IFD of that rank, typed UNDEFINED (a maker note).
    Note(usize),
}

pub(crate) type Ifd = Vec<(u16, V)>;

struct Bytes {
    little: bool,
}

impl Bytes {
    fn u16(&self, v: u16) -> [u8; 2] {
        if self.little {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        }
    }

    fn u32(&self, v: u32) -> [u8; 4] {
        if self.little {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        }
    }
}

fn ascii(text: &str) -> (u16, u32, Vec<u8>) {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    (2, data.len() as u32, data)
}

/// `(type, count, data)` of a value, `Ptr`s resolved with `starts`.
fn encode(bytes: &Bytes, value: &V, starts: &[u32]) -> (u16, u32, Vec<u8>) {
    match value {
        V::Ascii(text) => ascii(text),
        V::Text(text) => ascii(text),
        V::Short(values) => (
            3,
            values.len() as u32,
            values.iter().flat_map(|v| bytes.u16(*v)).collect(),
        ),
        V::Long(values) => (
            4,
            values.len() as u32,
            values.iter().flat_map(|v| bytes.u32(*v)).collect(),
        ),
        V::Rational(values) => (
            5,
            values.len() as u32,
            values
                .iter()
                .flat_map(|(n, d)| bytes.u32(*n).into_iter().chain(bytes.u32(*d)))
                .collect(),
        ),
        V::Ptr(rank) => (4, 1, bytes.u32(starts[*rank]).to_vec()),
        // The reader follows the pointer of a maker note: the value field
        // holds it and the count says « more than four bytes ».
        V::Note(rank) => (7, 16, bytes.u32(starts[*rank]).to_vec()),
    }
}

/// A TIFF stream: the header, then each IFD followed by its data.
pub(crate) fn tiff(little: bool, magic: u16, ifds: &[Ifd]) -> Vec<u8> {
    let bytes = Bytes { little };
    let size_of = |ifd: &Ifd| -> u32 {
        let data: u32 = ifd
            .iter()
            .map(|(_, v)| {
                let (_, _, data) = encode(&bytes, v, &vec![0; ifds.len()]);
                if matches!(v, V::Note(_)) || data.len() <= 4 {
                    0
                } else {
                    (data.len() as u32).div_ceil(2) * 2
                }
            })
            .sum();
        2 + 12 * ifd.len() as u32 + 4 + data
    };
    let mut starts = Vec::new();
    let mut at = 8_u32;
    for ifd in ifds {
        starts.push(at);
        at += size_of(ifd);
    }
    let mut out = Vec::new();
    out.extend(if little { b"II" } else { b"MM" });
    out.extend(bytes.u16(magic));
    out.extend(bytes.u32(starts[0]));
    for (rank, ifd) in ifds.iter().enumerate() {
        let mut head = Vec::new();
        let mut data = Vec::new();
        head.extend(bytes.u16(ifd.len() as u16));
        let data_start = starts[rank] + 2 + 12 * ifd.len() as u32 + 4;
        for (tag, value) in ifd {
            let (kind, count, encoded) = encode(&bytes, value, &starts);
            head.extend(bytes.u16(*tag));
            head.extend(bytes.u16(kind));
            head.extend(bytes.u32(count));
            if matches!(value, V::Note(_)) || encoded.len() <= 4 {
                let mut field = encoded.clone();
                field.resize(4, 0);
                head.extend(field);
            } else {
                head.extend(bytes.u32(data_start + data.len() as u32));
                data.extend(&encoded);
                if encoded.len() % 2 == 1 {
                    data.push(0);
                }
            }
        }
        head.extend([0, 0, 0, 0]);
        out.extend(head);
        out.extend(data);
        assert_eq!(out.len() as u32, starts[rank] + size_of(ifd));
    }
    out
}

/// What a test file says.
pub(crate) struct Shot {
    pub make: &'static str,
    pub model: &'static str,
    pub exposure: (u32, u32),
    pub iso: u16,
    /// `YYYY:MM:DD HH:MM:SS`, the camera clock.
    pub taken: String,
    /// `OffsetTimeOriginal`; `None`: the file names no zone.
    pub offset: Option<&'static str>,
    /// Padding after the metadata, standing for the sensor data.
    pub pixels: usize,
    pub temperature: Option<i16>,
}

impl Shot {
    /// A 30 s ISO 800 shot of a Nikon, the clock without a zone.
    pub(crate) fn new(taken: &str, exposure_seconds: u32, iso: u16) -> Self {
        Shot {
            make: "NIKON CORPORATION",
            model: "NIKON D3300",
            exposure: (exposure_seconds, 1),
            iso,
            taken: taken.to_string(),
            offset: None,
            pixels: 64,
            temperature: None,
        }
    }
}

/// Writes the file of `shot` at `path` (an NEF-shaped TIFF, whatever the
/// extension).
pub(crate) fn write_raw(path: &Path, shot: &Shot) {
    let main: Ifd = vec![
        (0x010F, V::Ascii(shot.make)),
        (0x0110, V::Ascii(shot.model)),
        (0x8769, V::Ptr(1)),
    ];
    let mut exif: Ifd = vec![
        (0x829A, V::Rational(vec![shot.exposure])),
        (0x8827, V::Short(vec![shot.iso])),
        (0x9003, V::Text(shot.taken.clone())),
    ];
    if let Some(offset) = shot.offset {
        exif.push((0x9011, V::Ascii(offset)));
    }
    let mut ifds = vec![main, exif];
    if let Some(degrees) = shot.temperature {
        let mut note = vec![0_u16; 16];
        note[12] = (i32::from(degrees) + 128) as u16;
        ifds[1].push((0x927C, V::Note(2)));
        ifds.push(vec![(0x0004, V::Short(note))]);
        ifds[0][0] = (0x010F, V::Ascii("Canon"));
    }
    let mut bytes = tiff(true, 0x2A, &ifds);
    bytes.extend(vec![0_u8; shot.pixels]);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}
