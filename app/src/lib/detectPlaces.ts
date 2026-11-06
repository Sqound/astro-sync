// « Détecter les sessions »: the core has no time-zone database, so the
// window gives the engine each place's offset from UTC, computed here from
// the place's own time zone (`Intl`).

import type { UserData } from "./bridge";

/** The offset of a time zone from UTC at an instant, minutes east of UTC
 * (+120 for Paris in summer), or `null` for a name the system does not know. */
export function utcOffsetMinutes(timeZone: string, at: Date): number | null {
  try {
    const parts = new Intl.DateTimeFormat("en-US", {
      timeZone,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
      second: "numeric",
    }).formatToParts(at);
    const value = (type: string) =>
      Number(parts.find((part) => part.type === type)?.value);
    const asUtc = Date.UTC(
      value("year"),
      value("month") - 1,
      value("day"),
      value("hour"),
      value("minute"),
      value("second"),
    );
    // The instant, cut to the second, as the zone's wall clock read as UTC.
    const instant = Math.floor(at.getTime() / 1000) * 1000;
    return Math.round((asUtc - instant) / 60000);
  } catch {
    return null;
  }
}

/** The person's data with the offset of each place that names a time zone
 * (a place without one keeps none: the engine cuts its nights at mean solar
 * noon). */
export function withUtcOffsets(user: UserData, at: Date): UserData {
  return {
    ...user,
    places: user.places.map((place) => ({
      ...place,
      utcOffsetMinutes: place.timeZone
        ? utcOffsetMinutes(place.timeZone, at)
        : null,
    })),
  };
}
