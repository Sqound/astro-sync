"""What the frames of a night bring to its stacking, read back from Siril's sequence file.

The poses stay weighted by their stars (``-weight=wfwhm``).  Re-stacking fourteen real nights with the other
weightings Siril offers showed none of them better everywhere (STACK-WEIGHT-01): weighting by the background noise
gave up to +29 % of signal to noise but lost 14 % on a night of thin clouds (a dimmed frame is a quiet one) and
widened the stars by up to 17 %; weighting by the number of stars lost 29 % on a short night; leaving out the
noisiest frames lost up to 4 %.  So nothing is decided here: what is read is said.

What is said (``weighting`` in the record of the stacking, one line in the log): how many frames were stacked, and
how many of them add more noise than they take away -- a frame whose background noise is over sqrt(2 + 1/n) times
the median of the n frames makes a mean of equal weights noisier (the bound follows from the variance of a mean,
it is no setting) -- with how many of those had a brighter sky than the median frame (dawn, Moon, cloud, veil).
"""

from __future__ import annotations

import math
import statistics
from pathlib import Path


def _floats(fields: list[str]) -> list[float] | None:
    try:
        return [float(value) for value in fields]
    except ValueError:
        return None


def harm_bound(frames: int) -> float:
    """The noise, as a multiple of the median, above which a frame makes a mean of ``frames`` equal frames noisier.

    A mean of n frames of noise s has a variance s^2 / n; one more frame of noise t gives (n s^2 + t^2) / (n + 1)^2,
    which is larger exactly when t^2 > (2 + 1/n) s^2.
    """
    return math.sqrt(2 + 1 / max(frames - 1, 1))


def read_weighting(sequence: Path) -> dict[str, object] | None:
    """What the frames of ``sequence`` (a stacked ``r_*.seq``) bring; ``None`` when the file says nothing usable."""
    try:
        lines = sequence.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return None
    selected: dict[int, bool] = {}
    noise: dict[int, list[float]] = {}
    background: list[float] = []
    image = 0
    for line in lines:
        fields = line.split()
        if not fields:
            continue
        head = fields[0]
        if head == "I" and len(fields) >= 3:
            selected[image] = fields[2] == "1"
            image += 1
        elif head.startswith("M") and "-" in head:
            # ``M<layer>-<image> ...``: the statistics of one layer of one image; the last figure is its
            # background noise.
            try:
                index = int(head.split("-", 1)[1])
            except ValueError:
                continue
            values = _floats(fields[1:])
            if values and values[-1] > 0:
                noise.setdefault(index, []).append(values[-1])
        elif head.startswith("R") and len(fields) >= 6:
            # ``R<layer> fwhm wfwhm roundness quality background stars ...``: the sky level of the frame.
            values = _floats(fields[1:6])
            background.append(values[4] if values else float("nan"))
    frames = [i for i in sorted(noise) if selected.get(i, True)]
    if len(frames) < 2:
        return None
    level = {i: statistics.fmean(noise[i]) for i in frames}
    typical = statistics.median(level.values())
    bound = harm_bound(len(frames))
    noisy = [i for i in frames if level[i] > bound * typical]
    skies = [background[i] for i in frames if i < len(background) and not math.isnan(background[i])]
    median_sky = statistics.median(skies) if skies else None
    brighter = [i for i in noisy if median_sky is not None and i < len(background) and background[i] > median_sky]
    return {
        "method": "wfwhm",
        "frames": len(frames),
        "noisy": len(noisy),
        "noisyBrightSky": len(brighter),
        "noisiest": round(max(level.values()) / typical, 2),
    }


def sentence(weighting: dict[str, object] | None) -> str | None:
    """The line of the log that says it."""
    if not weighting:
        return None
    frames, noisy = weighting["frames"], weighting["noisy"]
    if not noisy:
        return f"frames: {frames} stacked, weighted by their stars; none adds more noise than it takes away"
    return (f"frames: {frames} stacked, weighted by their stars; {noisy} add more noise than they take away "
            f"(background noise up to {weighting['noisiest']:.1f} times the median; {weighting['noisyBrightSky']} of "
            f"them under a brighter sky: dawn, Moon, cloud or veil)")
