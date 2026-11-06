// The pure logic of the hosted sign-in screens -- no React, no Tauri -- split
// out the same way `capabilities.ts` and `serviceWait.ts` are, so it runs under
// `node --test`.
//
// What the screens need beyond the bridge: a countdown in the form « 0:42 »,
// what a refusal of the code tells the person (wrong code and the tries that
// are left, expired, too many tries and when the wait ends, no connection,
// the service still starting), which screen an announcement calls for, and
// the sentence of the terms and the privacy policy cut where its links go.

import type { SignInAnnouncement, SignInEnd } from "./bridge";
import {
  DEFAULT_LOCALE,
  shippedLocaleFor,
  type TranslateParams,
} from "../i18n/locale.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** How long the code of a message serves, said on the screen (`CodeRequested`
 * carries the same 600 seconds). */
export const CODE_VALID_MINUTES = 10;

/** « 0:42 », « 14:52 »: a wait as a countdown. Never negative. */
export function formatCountdown(seconds: number): string {
  const total = Math.max(0, Math.ceil(seconds));
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  return `${minutes}:${rest.toString().padStart(2, "0")}`;
}

/** Whole seconds until `deadline` (ms since the epoch), never negative. */
export function secondsLeft(deadline: number, now: number): number {
  return Math.max(0, Math.ceil((deadline - now) / 1000));
}

/** « 42 s » under a minute and a half, « 15 min » up to an hour, « 24 h »
 * beyond, through the language's own units. The lock of a sign-in lasts 15
 * minutes, then 1 hour, then 24 hours: none is said as a count of minutes it
 * is not. */
export function waitText(t: Translate, seconds: number): string {
  if (seconds < 90) {
    return t("common.secondsShort", { count: Math.max(1, Math.ceil(seconds)) });
  }
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) {
    return t("common.minutesShort", { count: minutes });
  }
  return t("common.hoursShort", { count: Math.ceil(seconds / 3600) });
}

/** The `code` of a failure caught from a bridge call, or `null`. */
export function errorCode(error: unknown): string | null {
  if (typeof error === "object" && error !== null && "code" in error) {
    const code = (error as { code: unknown }).code;
    return typeof code === "string" ? code : null;
  }
  return null;
}

function errorDetails(error: unknown): Record<string, unknown> | null {
  if (typeof error === "object" && error !== null && "details" in error) {
    const details = (error as { details: unknown }).details;
    if (typeof details === "object" && details !== null) {
      return details as Record<string, unknown>;
    }
  }
  return null;
}

/** How long a refusal says to wait, in seconds: the `Retry-After` the
 * service sent (a count, so it does not depend on the two clocks agreeing),
 * else the distance to `retryAt`; `null` when it says nothing. */
export function retryAfterSeconds(error: unknown, now: number): number | null {
  const details = errorDetails(error);
  if (!details) {
    return null;
  }
  if (
    typeof details.retryAfterSeconds === "number" &&
    Number.isFinite(details.retryAfterSeconds)
  ) {
    return Math.max(0, Math.ceil(details.retryAfterSeconds));
  }
  if (typeof details.retryAt === "string") {
    const at = Date.parse(details.retryAt);
    if (!Number.isNaN(at)) {
      return Math.max(0, Math.ceil((at - now) / 1000));
    }
  }
  return null;
}

/** When a refusal says the person may try again, as an instant (ms since the
 * epoch) on this computer's clock: the `Retry-After` counted from now (a
 * count, so the two clocks need not agree), else the distance to the `retryAt`
 * the service sent; `null` when it says nothing. */
export function retryInstant(error: unknown, now: number): number | null {
  const seconds = retryAfterSeconds(error, now);
  return seconds === null ? null : now + seconds * 1000;
}

/** Up to two hours a wait is said as a distance (« dans 15 min »); past it as
 * the day and the time it ends (« demain, 1 oct., à 21:20 »), which is what a
 * person of a 24-hour lock wants to read. */
const SAID_AS_DISTANCE_SECONDS = 2 * 3600;

/** The instant `at` said as an adverbial phrase in the language of `t`: « dans
 * 15 min », « aujourd'hui à 21:35 », « demain, 1 oct., à 21:20 ». `timeZone`
 * is the machine's own unless a test names one. */
export function whenText(
  t: Translate,
  locale: string,
  at: number,
  now: number,
  timeZone?: string,
): string {
  const seconds = Math.max(0, Math.ceil((at - now) / 1000));
  if (seconds < SAID_AS_DISTANCE_SECONDS) {
    return t("space.when.in", { wait: waitText(t, seconds) });
  }
  const time = new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    timeZone,
  }).format(at);
  const day = (instant: number) =>
    new Intl.DateTimeFormat("en-CA", { timeZone }).format(instant);
  if (day(at) === day(now)) {
    return t("space.when.today", { time });
  }
  const date = new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    timeZone,
  }).format(at);
  const tomorrow = day(now + 24 * 3600 * 1000);
  return day(at) === tomorrow
    ? t("space.when.tomorrow", { date, time })
    : t("space.when.later", { date, time });
}

/** The instant a wait ends (from `retryInstant`, fixed when the refusal
 * came), said as `whenText` says it as time goes by -- **never a duration of
 * Astro Sync's own**: « un peu plus tard » when the service said nothing. */
export function untilText(
  t: Translate,
  locale: string,
  until: number | null,
  now: number,
  timeZone?: string,
): string {
  return until === null
    ? t("space.when.unknown")
    : whenText(t, locale, until, now, timeZone);
}

/** When a refusal says the person may try again, said as `whenText` says it,
 * from what the service sent (`retryAt`, `Retry-After`). */
export function retryWhenText(
  t: Translate,
  locale: string,
  error: unknown,
  now: number,
  timeZone?: string,
): string {
  return untilText(t, locale, retryInstant(error, now), now, timeZone);
}

/** What went wrong with a code: each state has its own sentence and its own
 * next step. */
export type CodeProblem =
  /** « Ce code ne correspond pas. » The code stays valid. */
  | { kind: "wrong"; attemptsRemaining: number | null }
  /** « Ce code a expiré. » (also used, replaced, or never asked) */
  | { kind: "expired" }
  /** « Trop d'essais. » The codes of the address cannot be entered; a new
   * one can be asked for when the service says (15 minutes, 1 hour or 24
   * hours). `seconds` is the wait it announced and `until` the instant it
   * ends on this computer's clock, fixed when the refusal came (a wait that
   * is said does not move with the clock); both are `null` when the service
   * says nothing -- never a duration of ours. */
  | { kind: "locked"; seconds: number | null; until: number | null }
  /** The space has its ceiling of devices. The address is proven and the
   * code used: detach a device, then ask for a new code. */
  | { kind: "deviceLimit" }
  /** « Pas de connexion à Internet. » Nothing was sent: the code stays valid. */
  | { kind: "offline" }
  /** The service did not finish starting within the wait: nothing was used. */
  | { kind: "startTimeout" }
  /** The code was sent and no answer came back: it may have been used. */
  | { kind: "unconfirmed" }
  /** Not six digits: caught before anything is sent. */
  | { kind: "malformed" }
  /** Too many attempts from this network. */
  | { kind: "rateLimited"; seconds: number | null; until: number | null }
  /** The person cancelled the wait. */
  | { kind: "cancelled" }
  /** The code is right, but the space of this address is no longer
   * available: nothing to retry, a new code would change nothing. */
  | { kind: "unavailable" }
  | { kind: "other"; error: unknown };

/** Reads a failure of `space_verify_code` as a [`CodeProblem`]. */
export function codeProblemOf(error: unknown, now: number): CodeProblem {
  switch (errorCode(error)) {
    case "sign_in_code_incorrect": {
      const attempts = errorDetails(error)?.attemptsRemaining;
      return {
        kind: "wrong",
        attemptsRemaining: typeof attempts === "number" ? attempts : null,
      };
    }
    case "sign_in_code_expired":
    case "sign_in_link_invalid":
      return { kind: "expired" };
    case "sign_in_locked":
      return {
        kind: "locked",
        seconds: retryAfterSeconds(error, now),
        until: retryInstant(error, now),
      };
    case "device_limit_reached":
      return { kind: "deviceLimit" };
    case "network_error":
      return { kind: "offline" };
    case "service_start_timeout":
      return { kind: "startTimeout" };
    case "service_write_unconfirmed":
      return { kind: "unconfirmed" };
    case "sign_in_code_malformed":
      return { kind: "malformed" };
    case "sign_in_rate_limited":
      return {
        kind: "rateLimited",
        seconds: retryAfterSeconds(error, now),
        until: retryInstant(error, now),
      };
    case "cancelled":
      return { kind: "cancelled" };
    case "space_unavailable":
      return { kind: "unavailable" };
    default:
      return { kind: "other", error };
  }
}

/** What a failure of the sign-in says, in the person's language. Every code
 * of the contract has a sentence of its own; **a code nobody knows yet
 * falls on the generic sentence, never on the raw text** the service or the
 * core produced. */
export function signInErrorText(
  t: Translate,
  error: unknown,
  now: number,
  locale = "en",
): string {
  const code = errorCode(error);
  if (code === null) {
    return t("space.genericRefusal");
  }
  switch (code) {
    // A lock lasts 15 minutes, 1 hour or 24 hours; a rate limit as long as
    // the service says: what is read is what the service sent (`retryAt`,
    // `Retry-After`), never a duration of ours.
    case "sign_in_locked":
    case "sign_in_rate_limited":
      return t(`errors.${code}`, {
        when: retryWhenText(t, locale, error, now),
      });
    // With `retryAt` the e-mail budget of the whole service is spent: the
    // sending of codes is suspended until then. Without it the settings of
    // the service are incomplete, and the message is the one it always was.
    case "sign_in_unavailable":
      return retryInstant(error, now) === null
        ? t("errors.sign_in_unavailable")
        : t("errors.sign_in_unavailable_until", {
            when: retryWhenText(t, locale, error, now),
          });
    case "service_write_unconfirmed":
      return t("space.requestUnconfirmed");
    case "portal_refused":
      return t("space.genericRefusal");
    case "invalid_portal_response":
      return t("space.unexpectedAnswer");
    default: {
      const key = `errors.${code}`;
      const text = t(key, { message: "" });
      return text === key ? t("space.genericRefusal") : text;
    }
  }
}

/** Whether what was typed can be an e-mail address: the service's own rule
 * (white space removed, one `@`, something before it, a dot in the domain
 * with something on each side). Only decides whether the button is live --
 * the service checks again, and says so if it disagrees. */
export function looksLikeEmail(typed: string): boolean {
  const email = typed.replace(/\s+/g, "");
  const parts = email.split("@");
  if (parts.length !== 2 || email.length > 254) {
    return false;
  }
  const [local, domain] = parts;
  const lastDot = domain.lastIndexOf(".");
  return local.length > 0 && lastDot > 0 && lastDot < domain.length - 1;
}

/** A private server signing in by e-mail code (`spaceCreation: "closed"`): a
 * portal of one person -- only its owner's address opens it, nothing is ever
 * created. It says « votre portail », never « votre espace Astro Portal ». */
export function isClosedPortal(
  announcement: SignInAnnouncement | null | undefined,
): boolean {
  return (
    announcement?.method === "email-code" &&
    announcement.spaceCreation === "closed"
  );
}

/** The screens a service's announcement can call for. */
export type SignInFlow = "email" | "pairing" | "none" | "unsupported";

export function flowOf(announcement: SignInAnnouncement): SignInFlow {
  switch (announcement.method) {
    case "email-code":
      return "email";
    case "pairing":
      return "pairing";
    case "none":
      return "none";
    default:
      return "unsupported";
  }
}

/** The address the sign-in form proposes: the one last typed, kept for «
 * Changer d'adresse » — never the address of a space that was deleted, which
 * the deletion promised to erase. */
export function emailToPropose(
  lastEmail: string,
  ended: { reason: SignInEnd } | null | undefined,
): string {
  return ended?.reason === "space_deleted" ? "" : lastEmail;
}

/** The sentence of the sign-in screen for a sign-in the service ended: the
 * dictionary key that says why -- with the words of a portal when the service
 * is one (`portal`). */
export function endedMessageKey(reason: SignInEnd, portal = false): string {
  switch (reason) {
    case "device_detached":
      return portal
        ? "space.portal.ended.deviceDetached"
        : "errors.device_detached";
    case "space_deleted":
      return "errors.space_deleted";
    default:
      // A newer sign-in replaced the token: the contract's own code.
      return "errors.device_authentication_required";
  }
}

/** Which sentence of consent to show: with both links, one, or none -- a
 * service that has configured neither has nothing to accept. */
export function legalTemplateKey(
  announcement: SignInAnnouncement,
):
  | "space.legal.both"
  | "space.legal.termsOnly"
  | "space.legal.privacyOnly"
  | null {
  if (announcement.method !== "email-code") {
    return null;
  }
  const terms = announcement.termsUrl !== null;
  const privacy = announcement.privacyUrl !== null;
  if (terms && privacy) {
    return "space.legal.both";
  }
  if (terms) {
    return "space.legal.termsOnly";
  }
  return privacy ? "space.legal.privacyOnly" : null;
}

/** The terms or the privacy policy for a reader of `locale`: the service's
 * template with `{lang}` replaced by the tag of the shown language (English
 * where the tag is none of the registry's), else the address the service
 * announced, which is the same page for everyone or the English one. `null`
 * where the service names no such link. */
export function legalAddress(
  announcement: SignInAnnouncement,
  kind: "terms" | "privacy",
  locale: string,
): string | null {
  if (announcement.method !== "email-code") {
    return null;
  }
  const address =
    kind === "terms" ? announcement.termsUrl : announcement.privacyUrl;
  const template =
    kind === "terms"
      ? announcement.termsUrlTemplate
      : announcement.privacyUrlTemplate;
  if (typeof template === "string" && template.includes("{lang}")) {
    const tag = shippedLocaleFor(locale) ?? DEFAULT_LOCALE;
    return template.split("{lang}").join(tag);
  }
  return address;
}

export type TemplatePart = { text: string } | { placeholder: string };

/** Cuts a translated template at its `{placeholder}`s, so a link can stand
 * where the language puts it (the order of the words is the translation's,
 * not the screen's). */
export function splitTemplate(template: string): TemplatePart[] {
  const parts: TemplatePart[] = [];
  let last = 0;
  for (const match of template.matchAll(/\{(\w+)\}/g)) {
    const start = match.index ?? 0;
    if (start > last) {
      parts.push({ text: template.slice(last, start) });
    }
    parts.push({ placeholder: match[1] });
    last = start + match[0].length;
  }
  if (last < template.length) {
    parts.push({ text: template.slice(last) });
  }
  return parts;
}

/** « portal.planastro.space »: the address of the space on the web, without
 * what a person need not read. */
export function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** The name of a language for the message the service sends: the shipped
 * locale the interface language asks for (`de`, `pt-BR`), English for a
 * language the registry does not ship. The service falls back to English on
 * its own for a tag it holds no message in. */
export function messageLanguage(locale: string): string {
  return shippedLocaleFor(locale.replace("_", "-")) ?? "en";
}

/**
 * The private server a person was typing when they left the sign-in card
 * (« J’utilise mon propre serveur »): kept for the life of the window, so
 * that going to another screen and coming back finds the address, not the
 * hosted card. `verified`: its address had been checked, so the card checks
 * it again on return; an address never checked is never contacted unasked.
 * Nothing is written to the disk: the address of a server is saved only
 * once the computer is connected to it.
 */
export interface OwnServerDraft {
  serverUrl: string;
  verified: boolean;
}

let currentOwnServerDraft: OwnServerDraft | null = null;

export const ownServerDraft = {
  remember(draft: OwnServerDraft): void {
    currentOwnServerDraft = { ...draft };
  },
  recall(): OwnServerDraft | null {
    return currentOwnServerDraft ? { ...currentOwnServerDraft } : null;
  },
  forget(): void {
    currentOwnServerDraft = null;
  },
};
