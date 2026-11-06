//! `astro-sync report`, the real binary, on a night whose guiding log keeps
//! the clock of the computer (+02:00) while the frames carry UTC: the clock
//! of the logs is found from the frames, as `send-logs` finds it, said in the
//! output, and the guiding is attributed to the frame it belongs to -- never
//! read at +00:00 in silence.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Temp(PathBuf);

impl Temp {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "astro-sync-report-clock-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn header(cards: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (keyword, value) in cards {
        let mut card = format!("{keyword:<8}= {value}").into_bytes();
        card.resize(80, b' ');
        out.extend_from_slice(&card);
    }
    let mut end = b"END".to_vec();
    end.resize(80, b' ');
    out.extend_from_slice(&end);
    out.resize(out.len().div_ceil(2880) * 2880, b' ');
    out
}

/// One 300 s frame at 21:00:00 UTC; the PHD2 log of the same minutes written
/// by a computer two hours east of UTC (23:00 on its clock).
fn night(dir: &Path) {
    fs::write(
        dir.join("PHD2_GuideLog_2026-09-20.txt"),
        "Guiding Begins at 2026-09-20 23:00:00\n\
         Pixel scale = 2.00 arc-sec/px\n\
         Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode\n\
         1,1.000,Mount,0,0,0.100,0.100,0,0,0,,0,,0,0,30000,12,0\n\
         2,3.000,Mount,0,0,0.200,0.100,0,0,0,,0,,0,0,30000,12,0\n\
         Guiding Ends at 2026-09-20 23:04:00\n",
    )
    .unwrap();
    fs::write(
        dir.join("frame-0001.fits"),
        header(&[
            ("EXPTIME", "300.0"),
            ("FILTER", "'Ha      '"),
            ("GAIN", "100"),
            ("DATE-OBS", "'2026-09-20T21:00:00.000'"),
            ("XBINNING", "1"),
            ("YBINNING", "1"),
        ]),
    )
    .unwrap();
}

fn report(folder: &Path, out: &Path, extra: &[&str]) -> (serde_json::Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .arg("report")
        .arg("--folder")
        .arg(folder)
        .arg("--out")
        .arg(out)
        .args(extra)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: serde_json::Value = serde_json::from_slice(&fs::read(out).unwrap()).unwrap();
    (
        written,
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn the_clock_of_the_logs_is_found_from_the_frames_and_said() {
    let dir = Temp::new("found");
    night(&dir.0);
    let out = dir.0.join("out.json");
    let (found, stdout) = report(&dir.0, &out.with_file_name("found.json"), &[]);
    let quality = &found["quality"][0];
    assert_eq!(quality["guidingSampleCount"], 2, "{found}");
    assert!(quality["rmsTotalArcsec"].as_f64().unwrap() > 0.0);
    assert!(
        stdout.contains("+02:00") && stdout.contains("found from the frames"),
        "the clock used is said: {stdout}"
    );
    // The same as the clock given by hand.
    let (given, said) = report(
        &dir.0,
        &out.with_file_name("given.json"),
        &["--phd2-utc-offset", "+02:00"],
    );
    assert_eq!(given["quality"], found["quality"]);
    assert!(said.contains("+02:00") && said.contains("given"), "{said}");
}
