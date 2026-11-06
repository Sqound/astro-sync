# Astro Sync: su carpeta de trabajo

Esta es la carpeta de trabajo de Astro Sync. Contiene sus noches de astrofotografía con la organización que Astro Sync sabe leer. Todo lo que hay en ella es un archivo o una carpeta normal: puede abrirlo, moverlo, modificarlo o eliminarlo usted mismo.

## Qué contiene

- Una carpeta por noche, con el nombre «fecha · plan o lugar · identificador corto», por ejemplo `2026-10-07 · Col des Étoiles · 7f3a2c`. Puede cambiarle el nombre: un archivo oculto conserva su identidad.
- Dentro de una noche:
  - `logs/`: los registros de NINA, PHD2 y ASIAIR de la noche;
  - `calibration/`: `darks/`, `flats/`, `dark-flats/` y `biases/`;
  - una carpeta por objetivo (`M31`, `NGC 7000`…), con `lights/` para las tomas FITS sin procesar y `final/` para su imagen procesada (TIFF, JPEG o PNG);
  - `stacked/` y `progression/`, que Astro Sync solo crea cuando fabrica una imagen por su cuenta.
- Los archivos y carpetas cuyo nombre empieza por un punto (`.astro-sync-night.json`, `.astro-sync-target.json`, `.astro-sync-work/`) son de Astro Sync: conservan la identidad de una noche o de un objetivo y el trabajo temporal de una tarea en curso. Déjelos donde están.

## Los archivos .md de la raíz

- `README.md`: este archivo.
- `ASSISTANT-IA.md`: el procedimiento que sigue un asistente de IA para ordenar los archivos de una noche. Está en inglés, sea cual sea su idioma.
- `GUIDE-ASTRO-SYNC.<idioma>.md` (por ejemplo `GUIDE-ASTRO-SYNC.es.md`): el texto de la página Guía de Astro Sync, escrito cuando usa «Hacer una pregunta a una IA» en la Guía. Hay un archivo por idioma solicitado.
- `<nombre>.modifie-<fecha>.md` (por ejemplo `ASSISTANT-IA.modifie-2026-10-17.md`): la copia de uno de estos archivos que usted había modificado, conservada cuando eligió «Sustituir por la versión de Astro Sync».

## Qué reescribe Astro Sync y qué no toca nunca

- Reescribe los archivos `.md` anteriores solo mientras estén exactamente como él los escribió, para que sigan las actualizaciones de la aplicación. Un archivo que usted modificó se deja tal cual: Astro Sync se lo dice y ofrece «Sustituir por la versión de Astro Sync», que primero renombra el suyo como `<nombre>.modifie-<fecha>.md`.
- Crea carpetas. Nunca mueve, renombra, modifica ni elimina sus tomas, sus registros, sus imágenes ni ningún archivo que usted haya copiado, movido o enlazado aquí. Sus propios archivos de procesamiento (masters, resultados del apilado, archivos temporales) solo se eliminan si usted lo pide, en Configuración › General › Archivos de procesamiento.

## No se envía nada a una IA

Astro Sync no contacta con ningún servicio de IA. «Preguntar a un asistente de IA» y «Hacer una pregunta a una IA» solo copian un texto en su portapapeles: usted se lo da al asistente que prefiera, que lee estos archivos con su propio acceso a su ordenador. Las tomas FITS y los registros sin procesar nunca salen de su ordenador; solo los resultados que usted elija enviar van a su espacio de Astro Portal.
