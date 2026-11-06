// The window's own storage (`localStorage`), for what Astro Sync keeps per
// computer between two openings of a screen: the rows of « Nuits » for the
// offline state, the targets left unticked at « Cibles ». It is a
// convenience: a storage that is empty or refuses to answer (a private
// window, cleared site data) just means starting again from the service.

/** The part of `Storage` the screens need; `window.localStorage` fits. */
export interface ReportStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** The window's storage, or `null` when the environment has none. */
export function windowStorage(): ReportStorage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}
