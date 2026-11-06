# Astro Sync: la sua cartella di lavoro

Questa è la cartella di lavoro di Astro Sync. Contiene le sue notti di astrofotografia nell’organizzazione che Astro Sync sa leggere. Tutto ciò che contiene è un normale file o una normale cartella: può aprirlo, spostarlo, modificarlo o eliminarlo da sé.

## Che cosa contiene

- Una cartella per notte, chiamata «data · piano o luogo · identificativo breve», per esempio `2026-10-07 · Col des Étoiles · 7f3a2c`. Può rinominarla: un file nascosto ne conserva l’identità.
- All’interno di una notte:
  - `logs/`: i log di NINA, PHD2 e ASIAIR della notte;
  - `calibration/`: `darks/`, `flats/`, `dark-flats/` e `biases/`;
  - una cartella per oggetto (`M31`, `NGC 7000`…), con `lights/` per i fotogrammi FITS grezzi e `final/` per la sua immagine elaborata (TIFF, JPEG o PNG);
  - `stacked/` e `progression/`, che Astro Sync crea solo quando produce da sé un’immagine.
- I file e le cartelle il cui nome inizia con un punto (`.astro-sync-night.json`, `.astro-sync-target.json`, `.astro-sync-work/`) sono di Astro Sync: conservano l’identità di una notte o di un oggetto e il lavoro temporaneo di un’operazione in corso. Li lasci dove sono.

## I file .md nella radice

- `README.md`: questo file.
- `ASSISTANT-IA.md`: la procedura che un assistente IA segue per sistemare i file di una notte. È in inglese, qualunque sia la sua lingua.
- `GUIDE-ASTRO-SYNC.<lingua>.md` (per esempio `GUIDE-ASTRO-SYNC.it.md`): il testo della pagina Guida di Astro Sync, scritto quando usa «Fare una domanda a un’IA» nella Guida. C’è un file per ogni lingua richiesta.
- `<nome>.modifie-<data>.md` (per esempio `ASSISTANT-IA.modifie-2026-10-17.md`): la copia di uno di questi file che lei aveva modificato, conservata quando ha scelto «Sostituisci con la versione di Astro Sync».

## Che cosa Astro Sync riscrive e che cosa non tocca mai

- Riscrive i file `.md` qui sopra solo finché sono esattamente come li ha scritti lui, in modo che seguano gli aggiornamenti dell’applicazione. Un file che lei ha modificato resta com’è: Astro Sync glielo dice e propone «Sostituisci con la versione di Astro Sync», che prima rinomina il suo in `<nome>.modifie-<data>.md`.
- Crea cartelle. Non sposta, non rinomina, non modifica e non elimina mai i suoi fotogrammi, i suoi log, le sue immagini né alcun file che lei ha copiato, spostato o collegato qui. I suoi file di elaborazione (master, risultati dello stacking, file temporanei) vengono eliminati solo se lei lo chiede, in Impostazioni › Generali › File di elaborazione.

## Nulla viene inviato a una IA

Astro Sync non contatta nessun servizio di IA. «Chiedere a un assistente IA» e «Fare una domanda a un’IA» si limitano a copiare un testo negli appunti: lei lo dà all’assistente che preferisce, che legge questi file con il proprio accesso al suo computer. I fotogrammi FITS e i log grezzi non lasciano mai il suo computer; solo i risultati che sceglie di inviare vanno nel suo spazio Astro Portal.
