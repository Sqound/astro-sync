import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  SUPPORTED_LOCALES,
  createTranslator,
  type Locale,
} from "../i18n/locale.ts";
import {
  codeProblemOf,
  emailToPropose,
  endedMessageKey,
  signInErrorText,
  flowOf,
  formatCountdown,
  hostOf,
  isClosedPortal,
  legalAddress,
  legalTemplateKey,
  looksLikeEmail,
  ownServerDraft,
  messageLanguage,
  retryAfterSeconds,
  retryInstant,
  retryWhenText,
  secondsLeft,
  splitTemplate,
  untilText,
  waitText,
  whenText,
} from "./space.ts";

const NOW = Date.parse("2026-09-29T21:20:08.000Z");

test("a wait is counted down as minutes and seconds, never negative", () => {
  assert.equal(formatCountdown(42), "0:42");
  assert.equal(formatCountdown(892), "14:52");
  assert.equal(formatCountdown(900), "15:00");
  assert.equal(formatCountdown(0), "0:00");
  assert.equal(formatCountdown(-5), "0:00");
  assert.equal(formatCountdown(59.2), "1:00");
  assert.equal(secondsLeft(NOW + 42_000, NOW), 42);
  assert.equal(secondsLeft(NOW - 1_000, NOW), 0);
});

test("a wait is said in seconds under a minute and a half, in minutes beyond", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const seconds = waitText(t, 42);
    const minutes = waitText(t, 892);
    assert.ok(seconds.includes("42"), `${locale}: ${seconds}`);
    assert.ok(minutes.includes("15"), `${locale}: ${minutes}`);
    assert.notEqual(seconds, minutes);
  }
  const fr = createTranslator("fr");
  assert.equal(waitText(fr, 42), "42 s");
  assert.equal(waitText(fr, 89), "89 s");
  assert.equal(waitText(fr, 90), "2 min");
  assert.equal(waitText(fr, 0), "1 s");
});

test("a refusal's wait is the Retry-After it sent, else the distance to retryAt", () => {
  assert.equal(
    retryAfterSeconds(
      {
        code: "sign_in_locked",
        details: { retryAfterSeconds: 892, retryAt: "2030-01-01T00:00:00Z" },
      },
      NOW,
    ),
    892,
    "a count wins over the date: the two clocks need not agree",
  );
  assert.equal(
    retryAfterSeconds(
      {
        code: "sign_in_locked",
        details: { retryAt: "2026-09-29T21:35:00.000Z" },
      },
      NOW,
    ),
    892,
  );
  assert.equal(
    retryAfterSeconds(
      { code: "x", details: { retryAt: "2020-01-01T00:00:00Z" } },
      NOW,
    ),
    0,
  );
  assert.equal(retryAfterSeconds({ code: "x" }, NOW), null);
  assert.equal(retryAfterSeconds("nope", NOW), null);
  assert.equal(
    retryAfterSeconds({ code: "x", details: { retryAt: "not a date" } }, NOW),
    null,
  );
});

test("every failure of a code is one of the states of the code step", () => {
  assert.deepEqual(
    codeProblemOf(
      {
        code: "sign_in_code_incorrect",
        message: "",
        details: { attemptsRemaining: 4 },
      },
      NOW,
    ),
    { kind: "wrong", attemptsRemaining: 4 },
  );
  assert.deepEqual(
    codeProblemOf({ code: "sign_in_code_incorrect", message: "" }, NOW),
    { kind: "wrong", attemptsRemaining: null },
  );
  assert.deepEqual(
    codeProblemOf({ code: "sign_in_code_expired", message: "" }, NOW),
    {
      kind: "expired",
    },
  );
  assert.deepEqual(
    codeProblemOf({ code: "sign_in_link_invalid", message: "" }, NOW),
    {
      kind: "expired",
    },
  );
  const locked = {
    code: "sign_in_locked",
    message: "",
    details: { retryAfterSeconds: 892 },
  };
  assert.deepEqual(codeProblemOf(locked, NOW), {
    kind: "locked",
    seconds: 892,
    until: NOW + 892_000,
  });
  // The service says nothing: no duration of ours is invented.
  const lockedWithoutWait = { code: "sign_in_locked", message: "" };
  assert.deepEqual(codeProblemOf(lockedWithoutWait, NOW), {
    kind: "locked",
    seconds: null,
    until: null,
  });
  assert.deepEqual(
    codeProblemOf({ code: "device_limit_reached", message: "" }, NOW),
    { kind: "deviceLimit" },
  );
  assert.deepEqual(codeProblemOf({ code: "network_error", message: "" }, NOW), {
    kind: "offline",
  });
  assert.deepEqual(
    codeProblemOf({ code: "service_start_timeout", message: "" }, NOW),
    {
      kind: "startTimeout",
    },
  );
  assert.deepEqual(
    codeProblemOf({ code: "service_write_unconfirmed", message: "" }, NOW),
    {
      kind: "unconfirmed",
    },
  );
  assert.deepEqual(
    codeProblemOf({ code: "sign_in_code_malformed", message: "" }, NOW),
    {
      kind: "malformed",
    },
  );
  const rateLimited = {
    code: "sign_in_rate_limited",
    message: "",
    details: { retryAfterSeconds: 1500 },
  };
  assert.deepEqual(codeProblemOf(rateLimited, NOW), {
    kind: "rateLimited",
    seconds: 1500,
    until: NOW + 1_500_000,
  });
  assert.deepEqual(codeProblemOf({ code: "cancelled", message: "" }, NOW), {
    kind: "cancelled",
  });
  assert.deepEqual(
    codeProblemOf({ code: "space_unavailable", message: "" }, NOW),
    {
      kind: "unavailable",
    },
  );
  const unknown = { code: "portal_refused", message: "x" };
  assert.deepEqual(codeProblemOf(unknown, NOW), {
    kind: "other",
    error: unknown,
  });
  assert.deepEqual(codeProblemOf(new Error("boom"), NOW).kind, "other");
});

test("the button of an address is live only for something that can be one", () => {
  for (const good of [
    "vous@exemple.fr",
    " Jean.Dupont+Astro@Exemple.FR ",
    "a@b.co",
    "vous @ exemple.fr",
  ]) {
    assert.ok(looksLikeEmail(good), good);
  }
  for (const bad of [
    "",
    "vous",
    "vous@",
    "@exemple.fr",
    "vous@exemple",
    "vous@.fr",
    "vous@exemple.",
    "a@b@c.fr",
  ]) {
    assert.ok(!looksLikeEmail(bad), bad);
  }
});

test("a service's announcement calls for one screen", () => {
  assert.equal(
    flowOf({
      method: "email-code",
      codeLength: 6,
      spaceCreation: "open",
      termsUrl: null,
      privacyUrl: null,
      termsUrlTemplate: null,
      privacyUrlTemplate: null,
    }),
    "email",
  );
  // A private server signing in by e-mail code calls for the same screens,
  // and the words of a portal (`isClosedPortal`).
  const closed = {
    method: "email-code",
    codeLength: 6,
    spaceCreation: "closed",
    termsUrl: null,
    privacyUrl: null,
    termsUrlTemplate: null,
    privacyUrlTemplate: null,
  } as const;
  assert.equal(flowOf(closed), "email");
  assert.equal(isClosedPortal(closed), true);
  assert.equal(isClosedPortal({ ...closed, spaceCreation: "open" }), false);
  assert.equal(isClosedPortal({ method: "pairing" }), false);
  assert.equal(isClosedPortal({ method: "none" }), false);
  assert.equal(isClosedPortal(null), false);
  assert.equal(flowOf({ method: "pairing" }), "pairing");
  assert.equal(flowOf({ method: "none" }), "none");
  assert.equal(
    flowOf({ method: "unsupported", announced: "passkey" }),
    "unsupported",
  );
});

test("the sentence of consent names only the links the service has configured", () => {
  const email = (termsUrl: string | null, privacyUrl: string | null) =>
    ({
      method: "email-code",
      codeLength: 6,
      spaceCreation: "open",
      termsUrl,
      privacyUrl,
      termsUrlTemplate: null,
      privacyUrlTemplate: null,
    }) as const;
  assert.equal(
    legalTemplateKey(email("https://t", "https://p")),
    "space.legal.both",
  );
  assert.equal(
    legalTemplateKey(email("https://t", null)),
    "space.legal.termsOnly",
  );
  assert.equal(
    legalTemplateKey(email(null, "https://p")),
    "space.legal.privacyOnly",
  );
  assert.equal(legalTemplateKey(email(null, null)), null);
  assert.equal(legalTemplateKey({ method: "pairing" }), null);
});

test("a template is cut where its links go, in every language", async () => {
  assert.deepEqual(splitTemplate("a {x} b {y}"), [
    { text: "a " },
    { placeholder: "x" },
    { text: " b " },
    { placeholder: "y" },
  ]);
  assert.deepEqual(splitTemplate("{x}"), [{ placeholder: "x" }]);
  assert.deepEqual(splitTemplate("plain"), [{ text: "plain" }]);
  // Each language's sentence keeps the places of its own links.
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = JSON.parse(
      await readFile(
        new URL(`../i18n/${locale as Locale}.json`, import.meta.url),
        "utf8",
      ),
    ) as Record<string, string>;
    const both = splitTemplate(dictionary["space.legal.both"]);
    const names = both.flatMap((part) =>
      "placeholder" in part ? [part.placeholder] : [],
    );
    assert.deepEqual([...names].sort(), ["privacy", "terms"], locale);
    assert.deepEqual(
      splitTemplate(dictionary["space.legal.termsOnly"]).flatMap((part) =>
        "placeholder" in part ? [part.placeholder] : [],
      ),
      ["terms"],
      locale,
    );
    assert.deepEqual(
      splitTemplate(dictionary["space.legal.privacyOnly"]).flatMap((part) =>
        "placeholder" in part ? [part.placeholder] : [],
      ),
      ["privacy"],
      locale,
    );
  }
});

test("the address of the space is shown without what a person need not read", () => {
  assert.equal(
    hostOf("https://portal.planastro.space"),
    "portal.planastro.space",
  );
  assert.equal(
    hostOf("https://portal.planastro.space/"),
    "portal.planastro.space",
  );
  assert.equal(hostOf("http://192.168.1.10:8788"), "192.168.1.10:8788");
  assert.equal(hostOf("not a url"), "not a url");
});

test("the message language is one the service knows", () => {
  assert.equal(messageLanguage("fr"), "fr");
  assert.equal(messageLanguage("ja-JP"), "ja");
  assert.equal(messageLanguage("en_GB"), "en");
  // The shipped languages ask for their own message.
  assert.equal(messageLanguage("de"), "de");
  assert.equal(messageLanguage("es-MX"), "es");
  assert.equal(messageLanguage("it_IT"), "it");
  assert.equal(messageLanguage("pt-BR"), "pt-BR");
  assert.equal(messageLanguage("pt_PT"), "pt-BR");
  // A language the registry does not ship reads English.
  assert.equal(messageLanguage("nl"), "en");
  assert.equal(messageLanguage(""), "en");
});

test("a failure of the sign-in is said plainly, and an unknown code never shows raw text", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const generic = t("space.genericRefusal");
    // A code nobody knows yet, or no code at all: the generic sentence. So
    // are the two the one-device-one-space rule answers a phone with, which
    // Astro Sync (`application: "sync"`) is never sent.
    for (const error of [
      { code: "brand_new_refusal", message: "The service's own English text." },
      {
        code: "device_attached_elsewhere",
        message: "The service's own English text.",
      },
      {
        code: "device_change_limited",
        message: "The service's own English text.",
      },
      { code: "portal_refused", message: "The service's own English text." },
      new Error("a raw exception text"),
      "just a string",
    ]) {
      const text = signInErrorText(t, error, NOW, locale);
      assert.equal(text, generic, `${locale}: ${JSON.stringify(error)}`);
      assert.ok(!text.includes("English") && !text.includes("raw"), text);
    }
    // Every refusal the contract knows has a sentence of its own.
    for (const code of [
      "email_invalid",
      "sign_in_code_malformed",
      "sign_in_code_incorrect",
      "sign_in_code_expired",
      "sign_in_link_invalid",
      "sign_in_locked",
      "sign_in_rate_limited",
      "email_delivery_failed",
      "sign_in_unavailable",
      "space_unavailable",
      "service_starting",
      "sign_in_wrong_service",
      "invalid_request",
      "not_astro_service",
      "incompatible_api_version",
      "network_error",
      "service_start_timeout",
      "cancelled",
      "device_detached",
      "space_deleted",
      "device_authentication_required",
      "device_limit_reached",
      // A private server signing in by e-mail code.
      "not_owner",
      "not_initialized",
    ]) {
      const text = signInErrorText(t, { code, message: "raw" }, NOW, locale);
      assert.notEqual(
        text,
        generic,
        `${locale}: ${code} falls on the generic sentence`,
      );
      assert.ok(!text.includes("errors."), `${locale}: ${code}: ${text}`);
      assert.ok(
        !text.includes("{"),
        `${locale}: ${code} leaves a placeholder: ${text}`,
      );
    }
  }
  const fr = createTranslator("fr");
  assert.equal(
    signInErrorText(
      fr,
      {
        code: "sign_in_locked",
        message: "",
        details: { retryAfterSeconds: 892 },
      },
      NOW,
      "fr",
    ),
    "Trop d’essais. Par sécurité, les codes de cette adresse ne peuvent plus être saisis pour le moment. Vous pourrez en demander un nouveau dans 15 min.",
  );
  assert.ok(
    signInErrorText(
      fr,
      { code: "space_unavailable", message: "" },
      NOW,
    ).includes("n’est plus disponible"),
  );
  assert.ok(
    signInErrorText(
      fr,
      { code: "service_write_unconfirmed", message: "" },
      NOW,
    ).includes("regardez votre boîte de réception"),
  );
});

test("a sign-in the service ended says why, in every language, and holds nothing of the space", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const sentences = (
      ["device_detached", "space_deleted", "connection_replaced"] as const
    ).map((reason) => t(endedMessageKey(reason)));
    assert.equal(
      new Set(sentences).size,
      3,
      `${locale}: three reasons, three sentences`,
    );
    for (const sentence of sentences) {
      assert.ok(
        !sentence.includes("errors.") && !sentence.includes("{"),
        sentence,
      );
    }
  }
  const fr = createTranslator("fr");
  assert.ok(fr(endedMessageKey("device_detached")).includes("détaché"));
  assert.ok(fr(endedMessageKey("space_deleted")).includes("supprimé"));
  assert.ok(fr(endedMessageKey("connection_replaced")).includes("remplacée"));
});

test("a private portal that is not yours or not installed yet is told plainly, in every language, as a portal", () => {
  const fr = createTranslator("fr");
  assert.equal(
    signInErrorText(fr, { code: "not_owner", message: "x" }, NOW, "fr"),
    "Ce portail appartient à quelqu’un d’autre. Utilisez l’adresse e-mail choisie à son installation.",
  );
  assert.equal(
    signInErrorText(fr, { code: "not_initialized", message: "x" }, NOW, "fr"),
    "Ce portail n’a pas encore de propriétaire. Terminez son installation depuis un navigateur.",
  );
  const en = createTranslator("en");
  assert.ok(
    signInErrorText(
      en,
      { code: "not_owner", message: "x" },
      NOW,
      "en",
    ).startsWith("This portal belongs to someone else."),
  );
  // The sentence of a sign-in the service ended is a portal's own.
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    assert.notEqual(
      t(endedMessageKey("device_detached", true)),
      t(endedMessageKey("device_detached")),
      locale,
    );
    assert.ok(!t(endedMessageKey("device_detached", true)).includes("{"));
  }
  assert.ok(
    fr(endedMessageKey("device_detached", true)).includes("votre portail"),
  );
  assert.ok(!fr(endedMessageKey("device_detached", true)).includes("espace"));
});

test("a wait of an hour or a day is said in hours, never as a count of minutes", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const hour = waitText(t, 3600);
    const day = waitText(t, 86_400);
    assert.ok(hour.includes("1"), `${locale}: ${hour}`);
    assert.ok(day.includes("24"), `${locale}: ${day}`);
    assert.ok(!day.includes("1440"), `${locale}: ${day}`);
  }
  const fr = createTranslator("fr");
  assert.equal(waitText(fr, 3540), "59 min");
  assert.equal(waitText(fr, 3600), "1 h");
  assert.equal(waitText(fr, 86_400), "24 h");
});

test("when a person may try again is what the service said: a distance for a short wait, the day and time for a long one", () => {
  const at = (iso: string) => Date.parse(iso);
  const now = at("2026-09-29T21:20:08.000Z");
  const fr = createTranslator("fr");
  const en = createTranslator("en");
  const ja = createTranslator("ja");
  // Up to two hours: a distance.
  assert.equal(whenText(fr, "fr", now + 892_000, now, "UTC"), "dans 15 min");
  assert.equal(whenText(en, "en", now + 892_000, now, "UTC"), "in 15 min");
  assert.equal(whenText(ja, "ja", now + 892_000, now, "UTC"), "15分後に");
  assert.equal(whenText(fr, "fr", now + 3_600_000, now, "UTC"), "dans 1 h");
  // Beyond: the day and the time it ends.
  assert.equal(
    whenText(fr, "fr", at("2026-09-29T23:35:00.000Z"), now, "UTC"),
    "à 23:35",
  );
  assert.equal(
    whenText(en, "en", at("2026-09-29T23:35:00.000Z"), now, "UTC"),
    "at 11:35 PM",
  );
  const tomorrow = whenText(fr, "fr", now + 86_400_000, now, "UTC");
  assert.ok(
    tomorrow.startsWith("demain, ") && tomorrow.endsWith("à 21:20"),
    tomorrow,
  );
  assert.ok(
    whenText(ja, "ja", now + 86_400_000, now, "UTC").startsWith("明日 "),
  );
  const later = whenText(fr, "fr", now + 3 * 86_400_000, now, "UTC");
  assert.ok(later.startsWith("le ") && later.endsWith(", à 21:20"), later);
  // The service's count wins over its date (the two clocks need not agree),
  // and a service that says nothing is not given a duration of ours.
  const locked = {
    code: "sign_in_locked",
    details: { retryAfterSeconds: 900, retryAt: "2031-01-01T00:00:00Z" },
  };
  assert.equal(retryInstant(locked, now), now + 900_000);
  assert.equal(retryWhenText(fr, "fr", locked, now, "UTC"), "dans 15 min");
  assert.equal(retryInstant({ code: "sign_in_locked" }, now), null);
  assert.equal(
    retryWhenText(fr, "fr", { code: "sign_in_locked" }, now, "UTC"),
    "un peu plus tard",
  );
});

test("a lock of a quarter hour, an hour or a day says when the codes can be asked for again, in every language", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const said = (seconds: number, at: string) =>
      signInErrorText(
        t,
        {
          code: "sign_in_locked",
          message: "The service's own English text.",
          details: { retryAfterSeconds: seconds, retryAt: at },
        },
        NOW,
        locale,
      );
    const quarter = said(900, "2026-09-29T21:35:08.000Z");
    const hour = said(3600, "2026-09-29T22:20:08.000Z");
    const day = said(86_400, "2026-09-30T21:20:08.000Z");
    assert.equal(new Set([quarter, hour, day]).size, 3, locale);
    for (const text of [quarter, hour, day]) {
      assert.ok(
        !text.includes("{") &&
          !text.includes("English") &&
          !text.includes("errors."),
        `${locale}: ${text}`,
      );
    }
    assert.ok(quarter.includes("15"), `${locale}: ${quarter}`);
    assert.ok(hour.includes("1"), `${locale}: ${hour}`);
    assert.ok(
      !day.includes("1440") && !day.includes("86400"),
      `${locale}: ${day}`,
    );
  }
  const fr = createTranslator("fr");
  assert.ok(
    signInErrorText(
      fr,
      { code: "sign_in_locked", details: { retryAfterSeconds: 3600 } },
      NOW,
      "fr",
    ).includes("dans 1 h"),
  );
  // Nothing said, nothing invented.
  const nothing = signInErrorText(fr, { code: "sign_in_locked" }, NOW, "fr");
  assert.ok(nothing.includes("un peu plus tard"), nothing);
  assert.ok(!nothing.includes("15"), nothing);
});

test("the sending of codes suspended by the service says so and when it ends; without a time, the message it always was", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const suspended = signInErrorText(
      t,
      {
        code: "sign_in_unavailable",
        message: "The service's own English text.",
        details: {
          retryAt: "2026-09-29T21:35:08.000Z",
          retryAfterSeconds: 900,
        },
      },
      NOW,
      locale,
    );
    const plain = signInErrorText(
      t,
      {
        code: "sign_in_unavailable",
        message: "The service's own English text.",
      },
      NOW,
      locale,
    );
    assert.notEqual(suspended, plain, locale);
    assert.ok(suspended.includes("15"), `${locale}: ${suspended}`);
    assert.ok(
      !suspended.includes("{") && !suspended.includes("English"),
      suspended,
    );
    assert.equal(plain, t("errors.sign_in_unavailable"), locale);
  }
  const fr = createTranslator("fr");
  assert.equal(
    signInErrorText(
      fr,
      { code: "sign_in_unavailable", details: { retryAfterSeconds: 900 } },
      NOW,
      "fr",
    ),
    "L’envoi des codes est momentanément suspendu. Vous pourrez réessayer dans 15 min.",
  );
  assert.equal(
    signInErrorText(fr, { code: "sign_in_unavailable" }, NOW, "fr"),
    "La connexion par e-mail n’est pas disponible sur ce service pour le moment.",
  );
});

test("a space at its ceiling of devices says to detach one from « Mes appareils » and to ask for a new code", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const text = signInErrorText(
      t,
      {
        code: "device_limit_reached",
        message: "The service's own English text.",
      },
      NOW,
      locale,
    );
    assert.ok(!text.includes("English") && !text.includes("{"), text);
    assert.equal(text, t("errors.device_limit_reached"), locale);
  }
  const fr = createTranslator("fr");
  const text = fr("errors.device_limit_reached");
  assert.ok(text.includes("Détachez un appareil"), text);
  assert.ok(text.includes("« Mes appareils »"), text);
  assert.ok(text.includes("application mobile ou sur le web"), text);
  assert.ok(text.includes("celui-ci a déjà servi"), text);
  const en = createTranslator("en")("errors.device_limit_reached");
  assert.ok(en.includes("Detach a device") && en.includes("My devices"), en);
  const ja = createTranslator("ja")("errors.device_limit_reached");
  assert.ok(ja.includes("切り離して") && ja.includes("マイデバイス"), ja);
});

test("a wait that was said counts down as time goes by, and a long one keeps the time it ends", () => {
  const fr = createTranslator("fr");
  const until = NOW + 900_000;
  assert.equal(untilText(fr, "fr", until, NOW, "UTC"), "dans 15 min");
  assert.equal(untilText(fr, "fr", until, NOW + 300_000, "UTC"), "dans 10 min");
  assert.equal(untilText(fr, "fr", until, NOW + 880_000, "UTC"), "dans 20 s");
  const day = NOW + 86_400_000;
  assert.equal(
    untilText(fr, "fr", day, NOW, "UTC"),
    untilText(fr, "fr", day, NOW + 60_000, "UTC"),
    "the day and the time do not move with the clock",
  );
  assert.equal(untilText(fr, "fr", null, NOW, "UTC"), "un peu plus tard");
});

test("the form proposes the address last typed, never the address of a deleted space", () => {
  // The deletion promised to erase the
  // address; a detachment keeps it as a convenience.
  assert.equal(emailToPropose("vous@exemple.fr", null), "vous@exemple.fr");
  assert.equal(emailToPropose("vous@exemple.fr", undefined), "vous@exemple.fr");
  assert.equal(
    emailToPropose("vous@exemple.fr", { reason: "device_detached" }),
    "vous@exemple.fr",
  );
  assert.equal(
    emailToPropose("vous@exemple.fr", { reason: "connection_replaced" }),
    "vous@exemple.fr",
  );
  assert.equal(
    emailToPropose("vous@exemple.fr", { reason: "space_deleted" }),
    "",
  );
});

test("the terms and the privacy policy open in the language the screens are shown in", () => {
  const announced = {
    method: "email-code",
    codeLength: 6,
    spaceCreation: "open",
    termsUrl: "https://www.example.test/en/terms/",
    privacyUrl: "https://www.example.test/en/privacy/",
    termsUrlTemplate: "https://www.example.test/{lang}/terms/",
    privacyUrlTemplate: "https://www.example.test/{lang}/privacy/",
  } as const;
  const expected: Record<string, string> = {
    fr: "fr",
    ja: "ja",
    de: "de",
    "zh-Hans": "zh-Hans",
    // A device spelling reads the shipped locale; an unknown language, English.
    "fr-CA": "fr",
    "zh-TW": "zh-Hant",
    xx: "en",
    "": "en",
  };
  for (const [locale, tag] of Object.entries(expected)) {
    assert.equal(
      legalAddress(announced, "terms", locale),
      `https://www.example.test/${tag}/terms/`,
      locale,
    );
    assert.equal(
      legalAddress(announced, "privacy", locale),
      `https://www.example.test/${tag}/privacy/`,
      locale,
    );
  }
  // A service announcing a plain address (or an older one, without the
  // templates) is the same page for everyone, as before.
  const plain = {
    ...announced,
    termsUrl: "https://www.example.test/terms",
    privacyUrl: null,
    termsUrlTemplate: null,
    privacyUrlTemplate: null,
  } as const;
  assert.equal(
    legalAddress(plain, "terms", "ja"),
    "https://www.example.test/terms",
  );
  assert.equal(legalAddress(plain, "privacy", "ja"), null);
  assert.equal(legalAddress({ method: "pairing" }, "terms", "fr"), null);
});

test("the private server being typed is found again after another screen, and forgotten once left", () => {
  ownServerDraft.forget();
  assert.equal(ownServerDraft.recall(), null);
  ownServerDraft.remember({
    serverUrl: "https://astro.example",
    verified: false,
  });
  assert.deepEqual(ownServerDraft.recall(), {
    serverUrl: "https://astro.example",
    verified: false,
  });
  // What is recalled is a copy: changing it changes nothing in the memory.
  const recalled = ownServerDraft.recall();
  if (recalled) recalled.serverUrl = "https://other.example";
  assert.equal(ownServerDraft.recall()?.serverUrl, "https://astro.example");
  ownServerDraft.remember({
    serverUrl: "https://astro.example",
    verified: true,
  });
  assert.equal(ownServerDraft.recall()?.verified, true);
  ownServerDraft.forget();
  assert.equal(ownServerDraft.recall(), null);
});
