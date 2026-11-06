# Astro Sync : votre dossier de travail

Voici le dossier de travail d’Astro Sync. Il contient vos nuits d’astrophotographie dans l’organisation qu’Astro Sync sait lire. Tout ce qu’il contient est un fichier ou un dossier ordinaire : vous pouvez l’ouvrir, le déplacer, le modifier ou le supprimer vous-même.

## Ce qu’il contient

- Un dossier par nuit, nommé « date · plan ou lieu · identifiant court », par exemple `2026-10-07 · Col des Étoiles · 7f3a2c`. Vous pouvez le renommer : un fichier caché garde son identité.
- Dans une nuit :
  - `logs/` : les journaux NINA, PHD2 et ASIAIR de la nuit ;
  - `calibration/` : `darks/`, `flats/`, `dark-flats/` et `biases/` ;
  - un dossier par cible (`M31`, `NGC 7000`…), avec `lights/` pour les poses FITS brutes et `final/` pour votre image traitée (TIFF, JPEG ou PNG) ;
  - `stacked/` et `progression/`, qu’Astro Sync ne crée que lorsqu’il fabrique lui-même une image.
- Les fichiers et dossiers dont le nom commence par un point (`.astro-sync-night.json`, `.astro-sync-target.json`, `.astro-sync-work/`) sont ceux d’Astro Sync : ils gardent l’identité d’une nuit ou d’une cible et le travail temporaire d’une tâche en cours. Laissez-les où ils sont.

## Les fichiers .md à la racine

- `README.md` : ce fichier.
- `ASSISTANT-IA.md` : la procédure qu’un assistant IA suit pour ranger les fichiers d’une nuit. Il est en anglais, quelle que soit votre langue.
- `GUIDE-ASTRO-SYNC.<langue>.md` (par exemple `GUIDE-ASTRO-SYNC.fr.md`) : le texte de la page Guide d’Astro Sync, écrit quand vous utilisez « Poser une question à une IA » dans le Guide. Il y a un fichier par langue demandée.
- `<nom>.modifie-<date>.md` (par exemple `ASSISTANT-IA.modifie-2026-10-17.md`) : la copie d’un de ces fichiers que vous aviez modifié, gardée quand vous avez choisi « Remplacer par la version d’Astro Sync ».

## Ce qu’Astro Sync réécrit, et ce qu’il ne touche jamais

- Il réécrit les fichiers `.md` ci-dessus seulement tant qu’ils sont exactement tels qu’il les a écrits, pour qu’ils suivent les mises à jour de l’application. Un fichier que vous avez modifié est laissé tel quel : Astro Sync vous le dit et propose « Remplacer par la version d’Astro Sync », qui commence par renommer le vôtre en `<nom>.modifie-<date>.md`.
- Il crée des dossiers. Il ne déplace, ne renomme, ne modifie ni ne supprime jamais vos poses, vos journaux, vos images ni aucun fichier que vous avez copié, déplacé ou lié ici. Ses propres fichiers de traitement (masters, résultats d’empilement, fichiers temporaires) ne sont supprimés que si vous le demandez, dans Réglages › Général › Fichiers de traitement.

## Rien n’est envoyé à une IA

Astro Sync ne contacte aucun service d’IA. « Demander à un assistant IA » et « Poser une question à une IA » se contentent de copier un texte dans votre presse-papiers : vous le donnez à l’assistant de votre choix, qui lit ces fichiers avec son propre accès à votre ordinateur. Les poses FITS et les journaux bruts ne quittent jamais votre ordinateur ; seuls les résultats que vous choisissez d’envoyer vont dans votre espace Astro Portal.
