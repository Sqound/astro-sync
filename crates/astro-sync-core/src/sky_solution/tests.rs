use super::*;
use crate::analysis::wcs::tan_cards;
use crate::raster::samples;
use crate::test_support::TempDir;
use std::cell::RefCell;

const SOLVED_AT: &str = "2026-10-08T21:30:00Z";

/// Header text, one card per line: a TAN solution centred on the image.
fn solved_header(width: u32, height: u32, ra: f64, dec: f64, scale: f64, rotation: f64) -> String {
    let crpix = (f64::from(width) / 2.0 + 0.5, f64::from(height) / 2.0 + 0.5);
    let mut cards = vec![
        ("NAXIS1", width.to_string()),
        ("NAXIS2", height.to_string()),
        ("SWCREATE", "'N.I.N.A. 3.1'".to_string()),
    ];
    cards.extend(tan_cards(ra, dec, crpix, scale, rotation));
    cards
        .iter()
        .map(|(key, value)| format!("{key:<8}= {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn header_of(text: &str) -> FitsHeader {
    parse_header_text(text)
}

fn size(width: u32, height: u32) -> ImageSize {
    ImageSize { width, height }
}

#[test]
fn a_solved_header_gives_the_fields_of_the_contract() {
    let header = header_of(&solved_header(4000, 3000, 10.6847, 41.2690, 1.2, 0.0));
    let solution = solution_from_header(
        &header,
        size(4000, 3000),
        RowOrder::BottomUp,
        SolutionSource::Header,
        Some("N.I.N.A. 3.1".to_string()),
        SOLVED_AT,
    )
    .unwrap();
    assert!((solution.ra_deg - 10.6847).abs() < 1e-5);
    assert!((solution.dec_deg - 41.2690).abs() < 1e-5);
    assert!((solution.pixel_scale_arcsec - 1.2).abs() < 1e-4);
    assert!(solution.rotation_deg.abs() < 1e-3);
    assert_eq!(solution.parity, Parity::Normal);
    assert!((solution.field_width_deg - 4000.0 * 1.2 / 3600.0).abs() < 1e-4);
    assert_eq!((solution.width, solution.height), (4000, 3000));
    assert_eq!(solution.schema_version, 1);

    // Every field the contract requires, and no other.
    let json = serde_json::to_value(&solution).unwrap();
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "decDeg",
            "fieldHeightDeg",
            "fieldWidthDeg",
            "height",
            "parity",
            "pixelScaleArcsec",
            "raDeg",
            "rotationDeg",
            "schemaVersion",
            "solvedAt",
            "solverVersion",
            "source",
            "width"
        ]
    );
    assert_eq!(json["source"], "header");
    assert_eq!(json["parity"], "normal");
}

#[test]
fn rotation_and_parity_follow_the_matrix_and_the_row_order() {
    let rotated = header_of(&solved_header(1000, 800, 200.0, -30.0, 2.0, 37.5));
    let up = solution_from_header(
        &rotated,
        size(1000, 800),
        RowOrder::BottomUp,
        SolutionSource::Astap,
        None,
        SOLVED_AT,
    )
    .unwrap();
    assert!((up.rotation_deg - 37.5).abs() < 1e-3);
    assert_eq!(up.parity, Parity::Normal);

    // The same matrix over rows numbered from the top: turned half a turn
    // and seen in a mirror.
    let down = solution_from_header(
        &rotated,
        size(1000, 800),
        RowOrder::TopDown,
        SolutionSource::Astap,
        None,
        SOLVED_AT,
    )
    .unwrap();
    assert!((down.rotation_deg - 217.5).abs() < 1e-3);
    assert_eq!(down.parity, Parity::Mirrored);

    // East to the right of north: mirrored.
    let mirrored =
        solved_header(1000, 800, 200.0, -30.0, 2.0, 0.0).replace("CD1_1   = -", "CD1_1   = ");
    let mirrored = solution_from_header(
        &header_of(&mirrored),
        size(1000, 800),
        RowOrder::BottomUp,
        SolutionSource::Astap,
        None,
        SOLVED_AT,
    )
    .unwrap();
    assert_eq!(mirrored.parity, Parity::Mirrored);
}

#[test]
fn a_header_written_for_the_image_is_read_without_a_solver() {
    let dir = TempDir::new("sky-header");
    let image = dir.path().join("M31_final.tif");
    let text = solved_header(3000, 2000, 10.68, 41.27, 1.5, 12.0);
    std::fs::write(&image, samples::tiff(3000, 2000, Some(&text))).unwrap();
    let solution = header_solution(&image, SOLVED_AT).unwrap();
    assert_eq!(solution.source, SolutionSource::Header);
    assert_eq!(solution.solver_version.as_deref(), Some("N.I.N.A. 3.1"));
    assert!((solution.rotation_deg - 12.0).abs() < 1e-3);

    // A crop: the header names another size, it is not this image's.
    std::fs::write(&image, samples::tiff(2900, 2000, Some(&text))).unwrap();
    assert_eq!(header_solution(&image, SOLVED_AT), None);

    // A `.wcs` beside a PNG.
    let png = dir.path().join("NGC7000.png");
    std::fs::write(&png, samples::png(3000, 2000)).unwrap();
    assert_eq!(header_solution(&png, SOLVED_AT), None);
    std::fs::write(png.with_extension("wcs"), &text).unwrap();
    assert!(header_solution(&png, SOLVED_AT).is_some());
}

#[cfg(unix)]
mod solvers {
    use super::*;
    use crate::external_process::fake_tool;

    /// A fake ASTAP: finds `-o`, writes `<out>.wcs` from `wcs` and records
    /// its arguments, then exits with `code`.
    fn fake_astap(dir: &Path, wcs: Option<&str>, code: i32, sleep: bool) -> SolverTool {
        let cards = dir.join("answer.wcs");
        if let Some(wcs) = wcs {
            std::fs::write(&cards, wcs).unwrap();
        }
        let args = dir.join("args.txt");
        let body = format!(
            "echo \"$@\" > '{args}'\n{sleep}out=''\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) out=\"$2\"; shift;; esac; shift; done\n{copy}exit {code}",
            args = args.display(),
            sleep = if sleep { "sleep 30\n" } else { "" },
            copy = if wcs.is_some() {
                format!("cp '{}' \"$out.wcs\"\n", cards.display())
            } else {
                String::new()
            },
        );
        SolverTool {
            path: fake_tool(dir, "astap", &body),
            version: Some("2026.10".to_string()),
            database: None,
        }
    }

    fn job<'a>(image: &'a Path, work: &'a Path, hints: SolveHints) -> SolveJob<'a> {
        SolveJob {
            image,
            hints,
            work_dir: work,
            timeout: Duration::from_secs(20),
            cancel: None,
            solved_at: SOLVED_AT,
        }
    }

    #[test]
    fn an_image_of_a_plan_is_solved_where_its_master_points_with_its_scale() {
        use crate::test_support::fits_bytes;
        let dir = TempDir::new("astap-plan-hints");
        // A night with its master (pointing and optics), and the folder of a
        // plan whose record names that night.
        let night = dir.path().join("2026-08-17 · n-17");
        let target = night.join("NGC 7000");
        std::fs::create_dir_all(target.join("stacked")).unwrap();
        std::fs::write(
            night.join(".astro-sync-night.json"),
            r#"{"format":1,"nightId":"n-17","serviceUrl":"https://s","createdAt":"2026-08-17T10:00:00Z"}"#,
        )
        .unwrap();
        std::fs::write(
            target.join(".astro-sync-target.json"),
            r#"{"format":1,"targetId":"openngc-ngc7000"}"#,
        )
        .unwrap();
        std::fs::write(
            target.join("stacked/nofilter_180s_g100_integration.fit"),
            fits_bytes(
                &[
                    ("NAXIS", "3"),
                    ("EXPTIME", "180.0"),
                    ("STACKCNT", "18"),
                    ("RA", "314.27499"),
                    ("DEC", "44.306946"),
                    ("XPIXSZ", "3.76"),
                    ("FOCALLEN", "190.0"),
                ],
                16,
            ),
        )
        .unwrap();
        let plan = dir.path().join("NGC 7000 · p-1");
        std::fs::create_dir_all(plan.join("stacked")).unwrap();
        std::fs::write(
            plan.join(".astro-sync-plan.json"),
            r#"{"format":1,"projectId":"p-1","targetId":"openngc-ngc7000","name":"NGC 7000"}"#,
        )
        .unwrap();
        std::fs::write(
            plan.join("stacked/NGC_7000_plan.json"),
            r#"{"masters":[{"nightId":"n-17"}]}"#,
        )
        .unwrap();
        let image = plan.join("stacked/NGC_7000_plan_raw.png");
        std::fs::write(&image, samples::png(2000, 1500)).unwrap();

        // No position, no scale given: the master's.
        let hints = hints_of_image(&image);
        assert_eq!(hints.ra_deg, Some(314.27499));
        let work = dir.path().join("data/solve/abc");
        let tool = fake_astap(
            dir.path(),
            Some(&solved_header(2000, 1500, 314.27, 44.3, 4.08, 0.0)),
            0,
            false,
        );
        solve_with_astap(&tool, &job(&image, &work, hints)).unwrap();
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert!(args.contains("-ra 20.951666"), "{args}");
        assert!(args.contains("-spd 134.306946"), "{args}");
        assert!(args.contains("-fov 1.7008"), "{args}");
        // An image of no known folder has none to give.
        let stray = dir.path().join("M31.png");
        std::fs::write(&stray, samples::png(10, 10)).unwrap();
        assert_eq!(hints_of_image(&stray), SolveHints::default());
    }

    #[test]
    fn astap_solves_into_astro_syncs_data_with_the_hints() {
        let dir = TempDir::new("astap-success");
        let image = dir.path().join("M31.jpg");
        std::fs::write(&image, samples::jpeg(4000, 3000)).unwrap();
        let work = dir.path().join("data/solve/abc");
        let tool = fake_astap(
            dir.path(),
            Some(&solved_header(4000, 3000, 10.68, 41.27, 1.2, 90.0)),
            0,
            false,
        );
        let hints = SolveHints {
            ra_deg: Some(10.68),
            dec_deg: Some(41.27),
            pixel_scale_arcsec: Some(1.2),
        };
        let solution = solve_with_astap(&tool, &job(&image, &work, hints)).unwrap();
        assert_eq!(solution.source, SolutionSource::Astap);
        assert_eq!(solution.solver_version.as_deref(), Some("ASTAP 2026.10"));
        assert!((solution.rotation_deg - 90.0).abs() < 1e-3);
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert!(args.contains("-ra 0.712000"), "{args}");
        assert!(args.contains("-spd 131.270000"), "{args}");
        assert!(args.contains("-fov 1.0000"), "{args}");
        assert!(args.contains(&format!("-o {}", work.join("astap").display())));
        // Nothing beside the image.
        let beside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("M31"))
            .collect();
        assert_eq!(beside, vec!["M31.jpg"]);
    }

    /// A FITS pose was never given to the solver (`image_size` only read
    /// TIFF, JPEG and PNG); the solver is now called, with the picture's size
    /// from `NAXIS1` × `NAXIS2`.
    #[test]
    fn astap_is_called_on_a_fits_pose() {
        let dir = TempDir::new("astap-fits");
        let image = dir.path().join("img-0028.fits");
        std::fs::write(&image, samples::fits(3072, 2080, &[("BZERO", "32768")])).unwrap();
        let work = dir.path().join("work");
        let tool = fake_astap(
            dir.path(),
            Some(&solved_header(3072, 2080, 331.29, 46.48, 1.24, 0.0)),
            0,
            false,
        );
        let hints = SolveHints {
            ra_deg: None,
            dec_deg: None,
            pixel_scale_arcsec: Some(1.2375),
        };
        let solution = solve_with_astap(&tool, &job(&image, &work, hints)).unwrap();
        assert_eq!((solution.width, solution.height), (3072, 2080));
        assert!((solution.ra_deg - 331.29).abs() < 1e-3);
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert!(args.contains(&format!("-f {}", image.display())), "{args}");
        // The field is the picture's height times the scale: 2080 x 1.2375".
        assert!(args.contains("-fov 0.7150"), "{args}");
    }

    #[test]
    fn astap_failures_are_said_in_words() {
        let dir = TempDir::new("astap-failures");
        let image = dir.path().join("IC1396.png");
        std::fs::write(&image, samples::png(2000, 2000)).unwrap();
        let work = dir.path().join("work");
        let few = fake_astap(dir.path(), None, 2, false);
        let failure =
            solve_with_astap(&few, &job(&image, &work, SolveHints::default())).unwrap_err();
        assert_eq!(failure, SolveFailure::TooFewStars);
        assert_eq!(failure.sentence(), "Too few stars recognised in the image.");
        let none = fake_astap(dir.path(), None, 1, false);
        assert_eq!(
            solve_with_astap(&none, &job(&image, &work, SolveHints::default())).unwrap_err(),
            SolveFailure::NotSolved
        );
        let no_database = fake_astap(dir.path(), None, 32, false);
        assert_eq!(
            solve_with_astap(&no_database, &job(&image, &work, SolveHints::default()))
                .unwrap_err()
                .code(),
            "no_star_database"
        );
    }

    /// A fake ASTAP that keeps a copy of every image it is given
    /// (`seen-<n>.fit`) and its arguments (`calls.txt`, one line a run), writes
    /// `ini` as its answer, and solves when `solves` exists beside it.
    fn recording_astap(dir: &Path, ini: &str) -> SolverTool {
        let answer = dir.join("answer.wcs");
        std::fs::write(&answer, solved_header(30, 20, 38.2, 61.45, 4.08, 55.0)).unwrap();
        let body = format!(
            "d='{dir}'\nn=$(ls \"$d\"/seen-*.fit 2>/dev/null | wc -l | tr -d ' ')\nf=''; out=''\necho \"$@\" >> \"$d/calls.txt\"\nwhile [ $# -gt 0 ]; do case \"$1\" in -f) f=\"$2\"; shift;; -o) out=\"$2\"; shift;; esac; shift; done\ncp \"$f\" \"$d/seen-$n.fit\"\nprintf '%s\\n' '{ini}' > \"$out.ini\"\nif [ -f \"$d/solves\" ]; then echo PLTSOLVD=T > \"$out.ini\"; cp '{answer}' \"$out.wcs\"; exit 0; fi\nexit 1",
            dir = dir.display(),
            answer = answer.display(),
        );
        SolverTool {
            path: fake_tool(dir, "astap", &body),
            version: Some("2026.09.01".to_string()),
            database: None,
        }
    }

    /// A night's stack as Astro Sync writes it: a Deflate RGB TIFF in
    /// `stacked/`, beside the integration master that holds the optics.
    fn deflate_stack(dir: &Path) -> PathBuf {
        let stacked = dir.join("stacked");
        std::fs::create_dir_all(&stacked).unwrap();
        let (width, height) = (64, 40);
        let pixels: Vec<u16> = (0..width * height * 3)
            .map(|i| 3000 + ((i * 37) % 200) as u16)
            .collect();
        let tiff = stacked.join("IC_1805_2026-08-09_raw.tif");
        std::fs::write(
            &tiff,
            input::tests::tiff16(width, height, 3, &pixels, 32946, 1, 7),
        )
        .unwrap();
        std::fs::write(
            stacked.join("nofilter_180s_g100_integration.fit"),
            input::tests::fits_float(width, height, &[vec![0.0; width * height]]),
        )
        .unwrap();
        tiff
    }

    /// ASTAP cannot read a Deflate TIFF and does not solve a deep stack as
    /// it is; it is given an uncompressed 16-bit FITS of the luminance,
    /// levelled then plain, with the target's position and the scale of the
    /// master beside the stack.
    #[test]
    fn astap_is_given_an_uncompressed_fits_of_a_deflate_stack_with_the_hints() {
        let dir = TempDir::new("astap-input");
        let image = deflate_stack(dir.path());
        let work = dir.path().join("work");
        let tool = recording_astap(dir.path(), "PLTSOLVD=F");
        let hints = SolveHints {
            ra_deg: Some(38.175),
            dec_deg: Some(61.45),
            pixel_scale_arcsec: None,
        };
        let failure = solve_with_astap(&tool, &job(&image, &work, hints)).unwrap_err();
        assert_eq!(failure, SolveFailure::NotSolved);
        assert_eq!(
            failure.sentence(),
            "The stars of the image match no field of the catalogue."
        );
        let calls = std::fs::read_to_string(dir.path().join("calls.txt")).unwrap();
        let calls: Vec<&str> = calls.lines().collect();
        assert_eq!(calls.len(), 2, "levelled, then plain: {calls:?}");
        assert!(calls[0].contains(&format!(
            "-f {} ",
            work.join("input-levelled.fit").display()
        )));
        assert!(calls[1].contains(&format!("-f {} ", work.join("input-plain.fit").display())));
        for call in &calls {
            assert!(
                call.contains("-ra 2.545000 -spd 151.450000 -r 30"),
                "{call}"
            );
            // 40 rows x 206.265 x 3.76 µm / 190 mm, from the master beside it.
            assert!(call.contains("-fov 0.0454"), "{call}");
            assert!(call.contains("-z 0 -log"), "{call}");
        }
        for seen in ["seen-0.fit", "seen-1.fit"] {
            let bytes = std::fs::read(dir.path().join(seen)).unwrap();
            let (header, offset) =
                crate::analysis::fits::parse_primary_header_with_data_offset(&bytes).unwrap();
            assert_eq!(header.get_i64("BITPIX"), Some(16), "{seen}");
            assert_eq!(header.get_i64("NAXIS"), Some(2), "{seen}");
            assert_eq!(
                (header.get_i64("NAXIS1"), header.get_i64("NAXIS2")),
                (Some(64), Some(40))
            );
            assert_eq!(header.get_f64("RA"), Some(38.175));
            assert_eq!(header.get_f64("DEC"), Some(61.45));
            assert!((header.get_f64("PIXSCALE").unwrap() - 4.0819).abs() < 1e-3);
            // Uncompressed: two bytes a pixel.
            assert!(bytes.len() - offset >= 64 * 40 * 2);
        }
        // The levelled image is not the plain one; neither stays behind.
        assert!(
            std::fs::read(dir.path().join("seen-0.fit")).unwrap()
                != std::fs::read(dir.path().join("seen-1.fit")).unwrap()
        );
        assert!(!work.join("input-levelled.fit").exists());

        // "Try again" runs the same path again: it solves now, and the
        // solution is the stack's, at its own size.
        std::fs::write(dir.path().join("solves"), "").unwrap();
        let solution = solve_with_astap(&tool, &job(&image, &work, hints)).unwrap();
        assert_eq!((solution.width, solution.height), (64, 40));
        assert_eq!(solution.source, SolutionSource::Astap);
    }

    /// "Field outside the installed indexes" only when ASTAP says its database
    /// does not suit the field; an unreadable image and too few stars keep
    /// their own words.
    #[test]
    fn each_end_of_astap_has_its_own_words() {
        let dir = TempDir::new("astap-words");
        let image = deflate_stack(dir.path());
        let work = dir.path().join("work");
        let wide = recording_astap(
            dir.path(),
            "PLTSOLVD=F\nWARNING=Large FOV, use G05 database!",
        );
        let failure =
            solve_with_astap(&wide, &job(&image, &work, SolveHints::default())).unwrap_err();
        assert_eq!(failure, SolveFailure::OutsideDatabase);
        assert_eq!(failure.code(), "outside_database");
        assert!(
            failure
                .sentence()
                .starts_with("Field outside the installed indexes")
        );
        // No second attempt: another image would not change the database.
        let calls = std::fs::read_to_string(dir.path().join("calls.txt")).unwrap();
        assert_eq!(calls.lines().count(), 1);

        // ASTAP ends with 1 on an image it cannot read, as on no solution:
        // its `ERROR=` line makes the difference (a Deflate TIFF given as it
        // was).
        let unreadable = recording_astap(dir.path(), "PLTSOLVD=F\nERROR=Error reading image file.");
        assert_eq!(
            solve_with_astap(&unreadable, &job(&image, &work, SolveHints::default())).unwrap_err(),
            SolveFailure::UnreadableImage
        );
        assert_eq!(
            serde_json::to_value(&failure).unwrap(),
            serde_json::json!({"code": "outside_database"})
        );

        let words: Vec<String> = [
            SolveFailure::UnreadableImage,
            SolveFailure::TooFewStars,
            SolveFailure::NotSolved,
            SolveFailure::OutsideDatabase,
            SolveFailure::NoDatabase,
        ]
        .iter()
        .map(SolveFailure::sentence)
        .collect();
        let mut distinct = words.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), words.len(), "{words:?}");
        assert_eq!(
            words
                .iter()
                .filter(|w| w.contains("installed indexes"))
                .count(),
            1
        );
    }

    /// Astrometry.net installed without a single index file stops with the
    /// engine's words; the person is told the cause.
    #[test]
    fn astrometry_net_without_index_files_says_so() {
        let dir = TempDir::new("astrometry-no-index");
        let image = dir.path().join("NGC7000.png");
        std::fs::write(&image, samples::png(3000, 2000)).unwrap();
        let work = dir.path().join("work");
        let engine_words = "Solving...\n\
             engine.c:82:engine_autoindex_search_paths: Warning: failed to open index directory: \"/usr/share/astrometry\"\n\
             You must list at least one index in the config file (/etc/astrometry.cfg)\n\
             solve-field.c:545:run_engine engine failed.\n \
             ioutils.c:568:run_command_get_outputs Command failed: return value 255";
        let body = format!("echo '{engine_words}'\nexit 255");
        let solvers = Solvers {
            astap: None,
            astrometry_net: Some(SolverTool {
                path: fake_tool(dir.path(), "solve-field", &body),
                version: None,
                database: None,
            }),
        };
        let failure = solve_image(
            &solvers,
            SolverChoice::AstrometryNet,
            &job(&image, &work, SolveHints::default()),
        )
        .unwrap_err();
        assert_eq!(failure, SolveFailure::NoIndex);
        assert_eq!(failure.code(), "no_index_files");
        let words = failure.sentence();
        assert!(words.contains("astrometry.net"), "{words}");
        assert!(words.contains("index"), "{words}");
        assert!(words.contains("Settings › Dependencies"), "{words}");
        assert!(!words.contains("255"), "{words}");

        // Any other stop keeps the solver's last words for the ⓘ.
        let other = fake_tool(
            dir.path(),
            "solve-field-crash",
            "echo 'out of memory'\nexit 3",
        );
        let solvers = Solvers {
            astap: None,
            astrometry_net: Some(SolverTool {
                path: other,
                version: None,
                database: None,
            }),
        };
        match solve_image(
            &solvers,
            SolverChoice::AstrometryNet,
            &job(&image, &work, SolveHints::default()),
        )
        .unwrap_err()
        {
            SolveFailure::Failed { detail } => assert!(detail.contains("out of memory")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn astap_past_its_time_is_stopped() {
        let dir = TempDir::new("astap-timeout");
        let image = dir.path().join("M31.png");
        std::fs::write(&image, samples::png(100, 100)).unwrap();
        let work = dir.path().join("work");
        let slow = fake_astap(dir.path(), None, 0, true);
        let mut job = job(&image, &work, SolveHints::default());
        job.timeout = Duration::from_millis(300);
        assert_eq!(
            solve_with_astap(&slow, &job).unwrap_err(),
            SolveFailure::Timeout { seconds: 0 }
        );
    }

    #[test]
    fn astrometry_net_when_chosen_and_the_header_before_any_solver() {
        let dir = TempDir::new("astrometry");
        let image = dir.path().join("NGC7000.png");
        std::fs::write(&image, samples::png(3000, 2000)).unwrap();
        let work = dir.path().join("work");
        let cards = dir.path().join("answer.wcs");
        std::fs::write(&cards, solved_header(3000, 2000, 314.7, 44.3, 3.0, 0.0)).unwrap();
        let body = format!(
            "dir=''\nwhile [ $# -gt 0 ]; do case \"$1\" in --dir) dir=\"$2\"; shift;; esac; shift; done\ncp '{}' \"$dir/astrometry.wcs\"",
            cards.display()
        );
        let solvers = Solvers {
            astap: Some(fake_astap(dir.path(), None, 1, false)),
            astrometry_net: Some(SolverTool {
                path: fake_tool(dir.path(), "solve-field", &body),
                version: None,
                database: None,
            }),
        };
        let solution = solve_image(
            &solvers,
            SolverChoice::AstrometryNet,
            &job(&image, &work, SolveHints::default()),
        )
        .unwrap();
        assert_eq!(solution.source, SolutionSource::AstrometryNet);
        // astrometry.net numbers the rows from the top.
        assert_eq!(solution.parity, Parity::Mirrored);
        // ASTAP (the default) answers « no solution ».
        assert_eq!(
            solve_image(
                &solvers,
                SolverChoice::Astap,
                &job(&image, &work, SolveHints::default())
            )
            .unwrap_err(),
            SolveFailure::NotSolved
        );
        // A header written for the image wins over any solver.
        std::fs::write(
            image.with_extension("wcs"),
            solved_header(3000, 2000, 314.7, 44.3, 3.0, 5.0),
        )
        .unwrap();
        assert_eq!(
            solve_image(
                &solvers,
                SolverChoice::Astap,
                &job(&image, &work, SolveHints::default())
            )
            .unwrap()
            .source,
            SolutionSource::Header
        );
        assert_eq!(
            solve_image(
                &Solvers::default(),
                SolverChoice::Astap,
                &job(&dir.path().join("none.png"), &work, SolveHints::default())
            )
            .unwrap_err(),
            SolveFailure::SolverMissing
        );
    }
}

struct FakeService {
    pages: Vec<serde_json::Value>,
    asked: RefCell<Vec<Option<String>>>,
    put: RefCell<Vec<(String, SkySolutionPut)>>,
}

impl SkySolutionTransport for FakeService {
    fn put_sky_solution(
        &self,
        _base_url: &str,
        _token: &str,
        capture_id: &str,
        body: &SkySolutionPut,
    ) -> Result<WrittenSkySolution, GuiBridgeError> {
        self.put
            .borrow_mut()
            .push((capture_id.to_string(), body.clone()));
        Ok(WrittenSkySolution {
            sky_solution: serde_json::to_value(&body.solution).unwrap(),
            created: true,
            changed: true,
        })
    }

    fn missing_page(
        &self,
        _base_url: &str,
        _token: &str,
        since: Option<&str>,
    ) -> Result<serde_json::Value, GuiBridgeError> {
        let mut asked = self.asked.borrow_mut();
        asked.push(since.map(str::to_string));
        Ok(self.pages[asked.len() - 1].clone())
    }
}

#[test]
fn the_missing_list_is_read_page_after_page_and_the_solution_is_put() {
    let capture = |id: &str, source: &str| {
        serde_json::json!({
            "id": id, "sessionId": null, "originalName": format!("{id}.jpg"),
            "sha256": "ab".repeat(32), "byteSize": 14_000_000, "mainImageId": format!("img-{id}"),
            "mainImageSource": source, "nightDate": "2026-10-07", "skySolved": false,
            "targets": [{"id": "t-m31", "source": "catalogue", "designation": "M31",
                         "raHours": 0.712, "decDegrees": 41.27}]
        })
    };
    let service = FakeService {
        pages: vec![
            serde_json::json!({"captures": [capture("c1", "phone")], "hasMore": true, "nextSince": "2026-10-08T00:00:00Z"}),
            serde_json::json!({"captures": [capture("c2", "web")], "hasMore": false, "nextSince": null}),
        ],
        asked: RefCell::new(Vec::new()),
        put: RefCell::new(Vec::new()),
    };
    let missing = list_missing(&service, "http://service.invalid", "token").unwrap();
    assert_eq!(missing.len(), 2);
    assert_eq!(missing[0].main_image_source.as_deref(), Some("phone"));
    assert_eq!(
        *service.asked.borrow(),
        vec![None, Some("2026-10-08T00:00:00Z".to_string())]
    );
    let hints = missing[0].hints();
    assert!((hints.ra_deg.unwrap() - 10.68).abs() < 1e-9);

    let header = header_of(&solved_header(100, 100, 10.0, 10.0, 1.0, 0.0));
    let solution = solution_from_header(
        &header,
        size(100, 100),
        RowOrder::BottomUp,
        SolutionSource::Astap,
        None,
        SOLVED_AT,
    )
    .unwrap();
    let written = send_solution(
        &service,
        "http://service.invalid",
        "token",
        "c1",
        &"AB".repeat(32),
        &solution,
    )
    .unwrap();
    assert!(written.created);
    let put = service.put.borrow();
    assert_eq!(put[0].0, "c1");
    assert_eq!(put[0].1.sha256, "ab".repeat(32));
    let body = serde_json::to_value(&put[0].1).unwrap();
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["sha256", "solution"]
    );
}

#[cfg(unix)]
#[test]
fn a_capture_from_the_phone_is_solved_from_its_original_and_sent() {
    use crate::external_process::fake_tool;
    use crate::night_resume::OriginalTransport;
    use sha2::{Digest, Sha256};

    struct Service {
        inner: FakeService,
        original: Vec<u8>,
    }
    impl SkySolutionTransport for Service {
        fn put_sky_solution(
            &self,
            base_url: &str,
            token: &str,
            capture_id: &str,
            body: &SkySolutionPut,
        ) -> Result<WrittenSkySolution, GuiBridgeError> {
            self.inner
                .put_sky_solution(base_url, token, capture_id, body)
        }
        fn missing_page(
            &self,
            base_url: &str,
            token: &str,
            since: Option<&str>,
        ) -> Result<serde_json::Value, GuiBridgeError> {
            self.inner.missing_page(base_url, token, since)
        }
    }
    impl OriginalTransport for Service {
        fn original(
            &self,
            _base_url: &str,
            _token: &str,
            _capture_id: &str,
            _image_id: &str,
            _from: u64,
        ) -> Result<(u16, Vec<u8>), GuiBridgeError> {
            Ok((200, self.original.clone()))
        }
    }

    let dir = TempDir::new("solve-and-send");
    let original = samples::png(2000, 1500);
    let sha = format!("{:x}", Sha256::digest(&original));
    let cards = dir.path().join("answer.wcs");
    std::fs::write(&cards, solved_header(2000, 1500, 10.68, 41.27, 2.0, 0.0)).unwrap();
    let wrote_at = dir.path().join("wrote-at.txt");
    let body = format!(
        "out=''\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) out=\"$2\"; shift;; esac; shift; done\ncp '{}' \"$out.wcs\"\necho \"$out\" >> '{}'",
        cards.display(),
        wrote_at.display()
    );
    let solvers = Solvers {
        astap: Some(SolverTool {
            path: fake_tool(dir.path(), "astap", &body),
            version: None,
            database: None,
        }),
        astrometry_net: None,
    };
    let service = Service {
        inner: FakeService {
            pages: Vec::new(),
            asked: RefCell::new(Vec::new()),
            put: RefCell::new(Vec::new()),
        },
        original,
    };
    let missing: MissingCapture = serde_json::from_value(serde_json::json!({
        "id": "c-phone", "originalName": "IMG_0042.png", "sha256": sha, "byteSize": 100,
        "mainImageId": "img-1", "mainImageSource": "phone", "targets": []
    }))
    .unwrap();
    let source = missing.solve_from(&std::collections::BTreeMap::new());
    assert!(matches!(source, SolveFrom::Original { .. }));
    let data = dir.path().join("data");
    let setup = SolveSetup {
        solvers: &solvers,
        choice: SolverChoice::Astap,
        originals_dir: &data.join("originals"),
        work_root: &data.join("solve"),
        timeout: Duration::from_secs(20),
        cancel: None,
        now: SOLVED_AT,
    };
    let outcome = solve_and_send(
        &service,
        "http://service.invalid",
        "token",
        &source,
        missing.hints(),
        &setup,
    )
    .unwrap();
    assert!(matches!(outcome, SolveOutcome::Sent { .. }), "{outcome:?}");
    assert_eq!(service.inner.put.borrow()[0].1.sha256, sha);
    assert!(data.join("originals").join(format!("{sha}.png")).exists());
    // The work of the resolution is gone.
    assert!(
        std::fs::read_to_string(&wrote_at)
            .unwrap()
            .contains(&data.join("solve").join(&sha).display().to_string())
    );
    assert!(!data.join("solve").join(&sha).exists());

    // The image of a night is solved on the night's disk, and the work goes
    // with the resolution.
    let night = dir.path().join("2026-08-10 · heart-nebula · ae7069");
    std::fs::create_dir_all(night.join("IC 1805/final")).unwrap();
    std::fs::write(night.join(".astro-sync-night.json"), "{}").unwrap();
    let image = night.join("IC 1805/final/ic1805.png");
    std::fs::write(&image, &service.original).unwrap();
    let here = std::collections::BTreeMap::from([(sha.clone(), image.clone())]);
    let local = missing.solve_from(&here);
    assert!(matches!(local, SolveFrom::Local { .. }));
    assert_eq!(
        solve_work_root(&local, &setup),
        night.join(".astro-sync-work/solve")
    );
    let outcome = solve_and_send(
        &service,
        "http://service.invalid",
        "token",
        &local,
        missing.hints(),
        &setup,
    )
    .unwrap();
    assert!(matches!(outcome, SolveOutcome::Sent { .. }), "{outcome:?}");
    let night_work = night.join(".astro-sync-work/solve").join(&sha);
    assert!(
        std::fs::read_to_string(&wrote_at)
            .unwrap()
            .contains(&night_work.display().to_string())
    );
    assert!(!night.join(".astro-sync-work").exists());
    assert!(image.exists());
    service.inner.put.borrow_mut().pop();

    // No solver: the row fails in words, nothing is put.
    let none = Solvers::default();
    let setup = SolveSetup {
        solvers: &none,
        ..setup
    };
    match solve_and_send(
        &service,
        "http://service.invalid",
        "token",
        &source,
        SolveHints::default(),
        &setup,
    )
    .unwrap()
    {
        SolveOutcome::Failed { failure, .. } => assert_eq!(failure, SolveFailure::SolverMissing),
        other => panic!("{other:?}"),
    }
    assert_eq!(service.inner.put.borrow().len(), 1);
}

#[test]
fn the_twin_of_a_finish_is_the_file_without_it_of_the_same_size() {
    let dir = TempDir::new("finish-twin");
    let stacked = dir.path().join("stacked");
    std::fs::create_dir_all(&stacked).unwrap();
    let write = |name: &str, width: u32| {
        let path = stacked.join(name);
        std::fs::write(&path, samples::tiff(width, 200, None)).unwrap();
        path
    };
    let plain = write("Sh2-103_plan_hoo.tif", 300);
    let finish = write("Sh2-103_plan_hoo_strong_colour-40_stars-85.tif", 300);
    let stars_only = write("Sh2-103_plan_hoo_stars-50.tif", 300);
    let curve = write("Sh2-103_plan_hoo_contrast-2.5.tif", 300);
    assert_eq!(finish_twin(&finish), Some(plain.clone()));
    assert_eq!(finish_twin(&stars_only), Some(plain.clone()));
    assert_eq!(finish_twin(&curve), Some(plain.clone()));
    // The image without a finish has no twin; nor has a finish whose twin is
    // another size (a cut image) or is not here.
    assert_eq!(finish_twin(&plain), None);
    let cut = write("Sh2-103_plan_hoo_framed_colour-20.tif", 150);
    assert_eq!(
        finish_twin(&cut),
        None,
        "no twin of the name: not the same size or not here"
    );
    let framed = write("Sh2-103_plan_raw_framed.tif", 150);
    let framed_finish = write("Sh2-103_plan_raw_framed_stars-85.tif", 280);
    assert_eq!(finish_twin(&framed_finish), None, "another size");
    assert_eq!(finish_twin(&framed), None);
}

/// The solvers fail on an image whose brightest stars were softened; the same
/// image without the finish is solved, and its solution is the finished
/// image's, put with the finished image's digest.
#[cfg(unix)]
#[test]
fn a_finished_image_takes_the_solution_of_its_twin_without_the_finish() {
    use crate::external_process::fake_tool;
    use crate::night_resume::OriginalTransport;

    struct Service(FakeService);
    impl SkySolutionTransport for Service {
        fn put_sky_solution(
            &self,
            base_url: &str,
            token: &str,
            capture_id: &str,
            body: &SkySolutionPut,
        ) -> Result<WrittenSkySolution, GuiBridgeError> {
            self.0.put_sky_solution(base_url, token, capture_id, body)
        }
        fn missing_page(
            &self,
            base_url: &str,
            token: &str,
            since: Option<&str>,
        ) -> Result<serde_json::Value, GuiBridgeError> {
            self.0.missing_page(base_url, token, since)
        }
    }
    impl OriginalTransport for Service {
        fn original(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: u64,
        ) -> Result<(u16, Vec<u8>), GuiBridgeError> {
            panic!("the image is here")
        }
    }

    let dir = TempDir::new("finish-twin-solve");
    let stacked = dir.path().join("plan/stacked");
    std::fs::create_dir_all(&stacked).unwrap();
    std::fs::write(
        stacked.join("Sh2-103_plan_hoo.tif"),
        samples::tiff(2000, 1500, None),
    )
    .unwrap();
    let finish = stacked.join("Sh2-103_plan_hoo_strong_colour-40_stars-85.tif");
    std::fs::write(&finish, samples::tiff(2000, 1500, None)).unwrap();
    let cards = dir.path().join("answer.wcs");
    std::fs::write(&cards, solved_header(2000, 1500, 312.93, 31.0, 4.08, 0.0)).unwrap();
    // The fake ASTAP finds nothing but in the work folder of the twin.
    let body = format!(
        "out=''\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) out=\"$2\"; shift;; esac; shift; done\ncase \"$out\" in *-twin*) cp '{}' \"$out.wcs\"; exit 0;; esac\nexit 1",
        cards.display()
    );
    let solvers = Solvers {
        astap: Some(SolverTool {
            path: fake_tool(dir.path(), "astap", &body),
            version: None,
            database: None,
        }),
        astrometry_net: None,
    };
    let data = dir.path().join("data");
    let setup = SolveSetup {
        solvers: &solvers,
        choice: SolverChoice::Astap,
        originals_dir: &data.join("originals"),
        work_root: &data.join("solve"),
        timeout: Duration::from_secs(20),
        cancel: None,
        now: SOLVED_AT,
    };
    let service = Service(FakeService {
        pages: Vec::new(),
        asked: RefCell::new(Vec::new()),
        put: RefCell::new(Vec::new()),
    });
    let source = SolveFrom::Local {
        capture_id: "c-veil".into(),
        image_id: "img-1".into(),
        sha256: "ab".repeat(32),
        path: finish.clone(),
    };
    let outcome = solve_and_send(
        &service,
        "http://service.invalid",
        "token",
        &source,
        SolveHints::default(),
        &setup,
    )
    .unwrap();
    assert!(matches!(outcome, SolveOutcome::Sent { .. }), "{outcome:?}");
    let put = service.0.put.borrow();
    assert_eq!(put[0].0, "c-veil");
    assert_eq!(
        put[0].1.sha256,
        "ab".repeat(32),
        "the finished image's own digest"
    );
    assert!((put[0].1.solution.ra_deg - 312.93).abs() < 0.01);
    assert_eq!(put[0].1.solution.width, 2000);
    // Nothing is left of the work of either attempt.
    assert!(!data.join("solve").join("ab".repeat(32)).exists());
}
