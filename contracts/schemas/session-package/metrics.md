# Session metrics and events

The column specification for `metrics/quality.csv` and `metrics/guiding.csv`,
and the line shape for `events/session.jsonl` (the three optional assets
`README.md` in this directory lists). This file is the public, importable column contract itself.

**Evidence rule (non-negotiable):** every
value here traces to a value a producer actually parsed from a source log or
FITS header. `rmsTotalArcsec`/`rmsRaArcsec`/`rmsDecArcsec` are a
root-mean-square of one directly recorded physical quantity (a guiding
offset, in arcseconds) -- a standard statistic in one unit, not a composite
rating that mixes unlike quantities into one opaque number. **No column or
JSON field named (or containing) `score`, `rating`, or `grade` may ever be
added to either file or to an event's `data` object.**

## Conventions shared by both CSV files

- **Encoding:** UTF-8, comma-separated, LF line endings, one header row.
  The header row is exact and asserted by producer tests -- an importer can
  rely on it never gaining, losing, or reordering a column without a
  version bump to this document.
- **Timestamps:** ISO 8601, UTC, millisecond precision,
  `"YYYY-MM-DDTHH:MM:SS.sssZ"` (always three fractional digits, always `Z`,
  never a bare offset). Millisecond precision (rather than the whole-second
  precision `manifest.json`'s own timestamps use) is deliberate: guide
  frames a fraction of a second apart must stay distinguishable.
- **Missing values:** an empty field. Never `null`, `0`, `NaN`, or a
  placeholder string -- a value that was not recorded or could not be
  parsed is represented by _absence_, exactly like `manifest.json`'s own
  nullable timestamps use `null` for "not applicable" at the JSON level.
- **Numbers:** plain decimal, fixed precision per column (below), rendered
  the same way on every run so two exports of the same source data are
  byte-identical. A CSV numeric field never carries a `+` sign, a thousands
  separator, or scientific notation.
- **Quoting:** RFC 4180 -- a field is wrapped in double quotes only when it
  contains a comma, a double quote (doubled inside), or a newline. Plain
  values (numbers, most filter names) are never quoted.

## `metrics/guiding.csv`

One row per raw guiding sample (one PHD2 guide-camera frame, or the
equivalent unit from another supported source) -- never aggregated. This is
the source evidence every guiding figure elsewhere is computed from.

| Column            | Unit                          | Precision | Meaning                                                                                                                                                                                                                                                                  |
| ----------------- | ----------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `timestampUtc`    | ISO 8601 UTC                  | ms        | When this sample was recorded.                                                                                                                                                                                                                                           |
| `source`          | enum string                   | --        | `phd2` or `asiair` (the parser that produced this row).                                                                                                                                                                                                                  |
| `frame`           | integer, dimensionless        | --        | The source log's own frame/sequence number.                                                                                                                                                                                                                              |
| `elapsedSeconds`  | s                             | 3         | The source log's own elapsed-time column (e.g. PHD2's `Time`), kept verbatim for traceability.                                                                                                                                                                           |
| `raOffsetArcsec`  | arcsec                        | 3         | Right-ascension guiding offset (signed): the raw measured star drift, converted from guide-camera pixels using the log's own declared pixel scale -- not the (possibly clamped) correction issued.                                                                       |
| `decOffsetArcsec` | arcsec                        | 3         | Declination guiding offset (signed), same convention as `raOffsetArcsec`.                                                                                                                                                                                                |
| `raDurationMs`    | ms                            | 0         | Guide-pulse duration actually issued on the RA axis for this frame (`0` when none was issued).                                                                                                                                                                           |
| `decDurationMs`   | ms                            | 0         | Guide-pulse duration actually issued on the Dec axis.                                                                                                                                                                                                                    |
| `starMass`        | dimensionless (source-native) | 0         | The guiding source's own star-mass/flux estimate, unit-less and specific to that source's internal scale -- never recalibrated or compared across sources.                                                                                                               |
| `snr`             | dimensionless                 | 2         | The guiding source's own signal-to-noise estimate for the guide star.                                                                                                                                                                                                    |
| `errorCode`       | integer, dimensionless        | --        | The source log's own per-frame error code (`0` = clean frame). Kept on every row, including non-zero ones: a consumer that wants root-mean-square guiding figures over only clean frames filters on this column itself, rather than the producer silently dropping rows. |

Header row (exact):

```text
timestampUtc,source,frame,elapsedSeconds,raOffsetArcsec,decOffsetArcsec,raDurationMs,decDurationMs,starMass,snr,errorCode
```

## `metrics/quality.csv`

One row per science sub-exposure ("pose"), built primarily from that
exposure's FITS header, plus the guiding root-mean-square aggregated from
`metrics/guiding.csv`'s raw samples that fall inside this exposure's time
window (`[DATE-OBS, DATE-OBS + exposureSeconds]`).

| Column               | Unit                           | Precision | Meaning                                                                                                                                                                   |
| -------------------- | ------------------------------ | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `timestampUtc`       | ISO 8601 UTC                   | ms        | Exposure start (`DATE-OBS`).                                                                                                                                              |
| `acquisitionFamily`  | string                         | --        | Stable label identifying this exposure's acquisition family (identical filter + exposure + gain + binning + rounded sensor temperature), e.g. `"Ha_300s_g100_1x1_t-10C"`. |
| `filter`             | string                         | --        | `FILTER` header value, verbatim.                                                                                                                                          |
| `exposureSeconds`    | s                              | 3         | `EXPTIME`/`EXPOSURE` header value.                                                                                                                                        |
| `gain`               | dimensionless (camera-native)  | 2         | `GAIN` header value (CMOS camera gain setting), when present.                                                                                                             |
| `iso`                | dimensionless (DSLR-native)    | 0         | `ISO` header value, when present. Mutually exclusive with `gain` in practice (one camera type or the other), both columns always present for a stable header shape.       |
| `binning`            | string (`"NxM"`)               | --        | `XBINNING`/`YBINNING`, e.g. `"1x1"`, `"2x2"`.                                                                                                                             |
| `sensorTempC`        | °C                             | 1         | `CCD-TEMP` header value.                                                                                                                                                  |
| `rmsTotalArcsec`     | arcsec                         | 3         | `hypot(rmsRaArcsec, rmsDecArcsec)` over this exposure's steady guiding samples (see below).                                                                               |
| `rmsRaArcsec`        | arcsec                         | 3         | Root-mean-square of `raOffsetArcsec` over this exposure's steady guiding samples.                                                                                         |
| `rmsDecArcsec`       | arcsec                         | 3         | Root-mean-square of `decOffsetArcsec` over this exposure's steady guiding samples.                                                                                        |
| `guidingSampleCount` | integer, dimensionless         | --        | How many `metrics/guiding.csv` samples the RMS columns above were computed from -- the evidence for how well-supported that figure is, never hidden.                      |
| `skyBackgroundAdu`   | ADU (sensor-native)            | 2         | `SKYBG`/`BACKGROUND` header value, exactly as recorded -- v1 applies no calibration (no conversion to mag/arcsec²); see "v1 scope" below.                                 |
| `fwhmPixels`         | px                             | 2         | `FWHM` header value, exactly as recorded (assumed already in pixels, the convention most capture/analysis tools use for this keyword; v1 applies no unit conversion).     |
| `eccentricity`       | dimensionless, [0, 1)          | 3         | `ECCENTRICITY`/`ECC` header value.                                                                                                                                        |
| `sourcePath`         | string (package-relative path) | --        | The exact FITS file this row was extracted from -- direct traceability from every other column back to its source file.                                                   |

**Steady guiding samples.** The RMS columns, like the slices of the session
report, are computed from the guiding samples that describe how well the mount
tracks, as PHD2 does: the frames taken while the guider settles after a dither
and while it walks back to its lock position, the first frame after a lost star,
the frames PHD2 gave an error code, those inside a meridian flip, those far
beyond the night's own noise (another star, a jump of the lock) and, when the
light frames are known, those taken outside their exposures are left out. They
all stay in `metrics/guiding.csv`.

Header row (exact):

```text
timestampUtc,acquisitionFamily,filter,exposureSeconds,gain,iso,binning,sensorTempC,rmsTotalArcsec,rmsRaArcsec,rmsDecArcsec,guidingSampleCount,skyBackgroundAdu,fwhmPixels,eccentricity,sourcePath
```

### v1 scope

Every FITS-derived column above is a direct pass-through of a header
keyword a capture or analysis tool already wrote -- v1 never computes
`fwhmPixels`, `eccentricity`, or `skyBackgroundAdu` from pixel data itself;
it reads them when a tool already recorded them and leaves the field empty
otherwise. Extracting these from raw pixel data is out of scope for this
version of the contract.

## `events/session.jsonl`

One compact JSON object per line (JSON Lines, UTF-8, LF-separated, one
trailing newline when the file is non-empty), each shaped:

```json
{
  "type": "star-lost",
  "timestampUtc": "2030-03-01T22:00:12.004Z",
  "source": "phd2",
  "data": { "statusText": "SNR too low", "precedingFrame": 5 }
}
```

| Field          | Meaning                                                                                  |
| -------------- | ---------------------------------------------------------------------------------------- |
| `type`         | Event type (see vocabulary below).                                                       |
| `timestampUtc` | ISO 8601 UTC, millisecond precision, same convention as the CSV files.                   |
| `source`       | `phd2` or `asiair`.                                                                      |
| `data`         | Event-specific fields (a JSON object, `{}` when the event itself carries no extra data). |

Object keys are written in sorted order (canonical JSON: no insignificant
whitespace, keys sorted at every nesting level) so two renderings of the
same event are byte-identical; a consumer must not rely on key order.

### v1 event vocabulary

| `type`             | `source` | `data` fields                                                                     | Meaning                                                                                                                                                     |
| ------------------ | -------- | --------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dither`           | `asiair` | `{}`                                                                              | A dither was commanded.                                                                                                                                     |
| `settle`           | `asiair` | `{}`                                                                              | ASIAIR reported a dither-settle checkpoint; the Autorun log format does not distinguish settle start from settle completion, so this event does not either. |
| `settle-timeout`   | `asiair` | `{}`                                                                              | Settling did not complete within the configured timeout.                                                                                                    |
| `dither-settle`    | `phd2`   | `{"outcome": "started"\|"complete"\|"failed", "precedingFrame": <int, nullable>}` | PHD2's own settle state change, which does distinguish start/complete/failed.                                                                               |
| `star-lost`        | `phd2`   | `{"statusText": <string, nullable>, "precedingFrame": <int, nullable>}`           | The guide star was lost.                                                                                                                                    |
| `pause`            | `phd2`   | `{"kind": "explicit-marker"\|"session-gap"}`                                      | Guiding paused (an explicit marker, or a gap between one guiding section ending and the next beginning in the same log file).                               |
| `resume`           | `phd2`   | `{"kind": "explicit-marker"\|"session-gap"}`                                      | Guiding resumed after a `pause`.                                                                                                                            |
| `autofocus`        | `asiair` | `{"temperatureC": <number, nullable>}`                                            | An autofocus run started.                                                                                                                                   |
| `autofocus-result` | `asiair` | `{"succeeded": <boolean>}`                                                        | An autofocus run finished.                                                                                                                                  |
| `meridian-flip`    | `asiair` | `{}`                                                                              | A meridian flip completed successfully.                                                                                                                     |

`precedingFrame` is `null` when the event occurred before any frame in its
session (or when the source frame number is otherwise unknown) --
PHD2's plain-text guide log does not attach an independent timestamp to
these inline markers the way it does to frame data rows (a `Time` column
elapsed since `Guiding Begins at`); a marker's `timestampUtc` is derived
from the most recently parsed frame before it, and `precedingFrame` makes
that approximation visible rather than presenting it as exact. The open
point is to validate this approximation against a real PHD2 log.

Future event types are additive (a new `type` value, never a meaning change
for an existing one); a consumer must ignore an unrecognized `type` rather
than reject the file.
