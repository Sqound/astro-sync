import { BrandLoader } from "./BrandLoader";

/**
 * What a screen, a sheet or a row that waits shows: the star, and a sentence that says what is awaited
 * (« Lecture de la nuit… »), in the body's size -- not the grey help text.
 * `page` centres a 64 px star over its sentence in the zone that waits;
 * `inline` puts a 22 px star beside its sentence, in a row or a button.
 */
export function Waiting({
  label,
  size = "page",
}: {
  label: string;
  size?: "page" | "inline";
}) {
  return (
    <div
      className={`as-waiting as-waiting--${size}`}
      role="status"
      aria-live="polite"
    >
      <BrandLoader
        className="as-waiting__star"
        size={size === "page" ? 64 : 22}
      />
      <p className="as-waiting__text">{label}</p>
    </div>
  );
}
