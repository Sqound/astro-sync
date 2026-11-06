# Astro Sync: dein Arbeitsordner

Dies ist der Arbeitsordner von Astro Sync. Er enthält deine astrofotografischen Nächte in der Anordnung, die Astro Sync lesen kann. Alles darin ist eine gewöhnliche Datei oder ein gewöhnlicher Ordner: Du kannst sie selbst öffnen, verschieben, bearbeiten oder löschen.

## Was darin liegt

- Ein Ordner pro Nacht, benannt „Datum · Plan oder Ort · kurze Kennung“, zum Beispiel `2026-10-07 · Col des Étoiles · 7f3a2c`. Du kannst ihn umbenennen: Eine versteckte Datei bewahrt seine Identität.
- In einer Nacht:
  - `logs/`: die NINA-, PHD2- und ASIAIR-Logs der Nacht;
  - `calibration/`: `darks/`, `flats/`, `dark-flats/` und `biases/`;
  - ein Ordner pro Ziel (`M31`, `NGC 7000` …) mit `lights/` für die rohen FITS-Aufnahmen und `final/` für dein bearbeitetes Bild (TIFF, JPEG oder PNG);
  - `stacked/` und `progression/`, die Astro Sync nur anlegt, wenn es selbst ein Bild erzeugt.
- Dateien und Ordner, deren Name mit einem Punkt beginnt (`.astro-sync-night.json`, `.astro-sync-target.json`, `.astro-sync-work/`), gehören Astro Sync: Sie bewahren die Identität einer Nacht oder eines Ziels und die Zwischenablage einer laufenden Aufgabe. Lass sie, wo sie sind.

## Die .md-Dateien im Hauptordner

- `README.md`: diese Datei.
- `ASSISTANT-IA.md`: der Ablauf, dem ein KI-Assistent folgt, um die Dateien einer Nacht zu ordnen. Sie ist auf Englisch, unabhängig von deiner Sprache.
- `GUIDE-ASTRO-SYNC.<Sprache>.md` (zum Beispiel `GUIDE-ASTRO-SYNC.de.md`): der Text der Seite „Anleitung“ von Astro Sync, geschrieben, wenn du in der Anleitung „Eine KI etwas fragen“ benutzt. Es gibt eine Datei pro gewünschter Sprache.
- `<Name>.modifie-<Datum>.md` (zum Beispiel `ASSISTANT-IA.modifie-2026-10-17.md`): die Kopie einer dieser Dateien, die du geändert hattest, aufbewahrt, als du „Durch die Version von Astro Sync ersetzen“ gewählt hast.

## Was Astro Sync neu schreibt und was es nie anfasst

- Es schreibt die oben genannten `.md`-Dateien nur neu, solange sie genau so sind, wie es sie geschrieben hat, damit sie den Aktualisierungen der Anwendung folgen. Eine Datei, die du geändert hast, bleibt, wie sie ist: Astro Sync sagt es dir und bietet „Durch die Version von Astro Sync ersetzen“ an, das zuerst deine Datei in `<Name>.modifie-<Datum>.md` umbenennt.
- Es legt Ordner an. Es verschiebt, benennt um, ändert oder löscht nie deine Aufnahmen, deine Logs, deine Bilder oder eine Datei, die du hier hineinkopiert, verschoben oder verlinkt hast. Seine eigenen Verarbeitungsdateien (Master, Stacking-Ergebnisse, temporäre Dateien) werden nur gelöscht, wenn du es verlangst, unter Einstellungen › Allgemein › Verarbeitungsdateien.

## Nichts wird an eine KI gesendet

Astro Sync kontaktiert keinen KI-Dienst. „KI-Assistenten fragen“ und „Eine KI etwas fragen“ kopieren nur einen Text in deine Zwischenablage: Du gibst ihn dem Assistenten deiner Wahl, der diese Dateien mit seinem eigenen Zugriff auf deinen Computer liest. Die FITS-Aufnahmen und die rohen Logs verlassen deinen Computer nie; nur die Ergebnisse, die du senden möchtest, gelangen in deinen Astro-Portal-Bereich.
