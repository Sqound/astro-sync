// Why a read failed, said in the language of the window. Pure and free of
// the bridge, so that `node --test` loads it (`format.ts` imports the bridge).

import type { TranslateParams } from "../i18n";

type Translate = (key: string, params?: TranslateParams) => string;

/** Why a read failed, for a place that writes "…: {reason}" after its own
 * sentence: the sentence of the error's code in the window's language
 * (`errors.<code>`, with the core's `message` as `{message}`), and the
 * core's own `message` only when the dictionary has no sentence for that
 * code -- or when the failure is no `{ code, message }` at all. */
export function errorReason(t: Translate, error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    typeof (error as { code?: unknown }).code === "string" &&
    typeof (error as { message?: unknown }).message === "string"
  ) {
    const { code, message } = error as { code: string; message: string };
    const key = `errors.${code}`;
    const sentence = t(key, { message });
    // A key the dictionary does not define is answered with the key itself.
    return sentence === key ? message : sentence;
  }
  return error instanceof Error ? error.message : String(error);
}
