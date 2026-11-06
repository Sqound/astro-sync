/** The device-language matching rule, independent of any runtime registry. */
export interface DeviceLocaleEntry {
  readonly tag: string;
  readonly language: string;
  readonly script?: string;
  readonly region?: string;
  readonly deviceRegions?: readonly string[];
  readonly deviceDefault?: boolean;
}

export function matchDeviceLocaleTag(
  candidate: string,
  entries: readonly DeviceLocaleEntry[],
): string | null {
  const parts = String(candidate ?? "")
    .trim()
    .replace(/_/g, "-")
    .split("-")
    .filter(Boolean);
  if (parts.length === 0) return null;
  const lower = parts.join("-").toLowerCase();
  const exact = entries.find((entry) => entry.tag.toLowerCase() === lower);
  if (exact) return exact.tag;
  const language = (parts[0] ?? "").toLowerCase();
  const script = parts.slice(1).find((part) => /^[A-Za-z]{4}$/.test(part));
  const region = parts
    .slice(1)
    .find((part) => /^(?:[A-Za-z]{2}|\d{3})$/.test(part))
    ?.toUpperCase();
  const candidates = entries.filter((entry) => entry.language === language);
  if (script) {
    const scripted = candidates.find(
      (entry) => entry.script?.toLowerCase() === script.toLowerCase(),
    );
    if (scripted) return scripted.tag;
  }
  if (region) {
    const regional = candidates.find(
      (entry) =>
        entry.deviceRegions?.includes(region) === true ||
        (entry.script === undefined && entry.region === region),
    );
    if (regional) return regional.tag;
  }
  const bare = candidates.find(
    (entry) => entry.script === undefined && entry.region === undefined,
  );
  if (bare) return bare.tag;
  return (
    (candidates.find((entry) => entry.deviceDefault === true) ?? candidates[0])
      ?.tag ?? null
  );
}
