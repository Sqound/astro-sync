# Astro Sync: a sua pasta de trabalho

Esta é a pasta de trabalho do Astro Sync. Ela guarda as suas noites de astrofotografia na organização que o Astro Sync sabe ler. Tudo o que há nela é um arquivo ou uma pasta comum: você mesmo pode abri-lo, movê-lo, editá-lo ou apagá-lo.

## O que ela contém

- Uma pasta por noite, chamada “data · plano ou local · identificador curto”, por exemplo `2026-10-07 · Col des Étoiles · 7f3a2c`. Você pode renomeá-la: um arquivo oculto guarda a identidade dela.
- Dentro de uma noite:
  - `logs/`: os registros do NINA, do PHD2 e do ASIAIR da noite;
  - `calibration/`: `darks/`, `flats/`, `dark-flats/` e `biases/`;
  - uma pasta por alvo (`M31`, `NGC 7000`…), com `lights/` para os quadros FITS brutos e `final/` para a sua imagem processada (TIFF, JPEG ou PNG);
  - `stacked/` e `progression/`, que o Astro Sync só cria quando produz ele mesmo uma imagem.
- Os arquivos e pastas cujo nome começa com um ponto (`.astro-sync-night.json`, `.astro-sync-target.json`, `.astro-sync-work/`) são do Astro Sync: guardam a identidade de uma noite ou de um alvo e o trabalho temporário de uma tarefa em andamento. Deixe-os onde estão.

## Os arquivos .md na raiz

- `README.md`: este arquivo.
- `ASSISTANT-IA.md`: o procedimento que um assistente de IA segue para organizar os arquivos de uma noite. Ele está em inglês, qualquer que seja o seu idioma.
- `GUIDE-ASTRO-SYNC.<idioma>.md` (por exemplo `GUIDE-ASTRO-SYNC.pt-BR.md`): o texto da página Guia do Astro Sync, escrito quando você usa “Fazer uma pergunta a uma IA” no Guia. Há um arquivo por idioma pedido.
- `<nome>.modifie-<data>.md` (por exemplo `ASSISTANT-IA.modifie-2026-10-17.md`): a cópia de um desses arquivos que você havia modificado, guardada quando você escolheu “Substituir pela versão do Astro Sync”.

## O que o Astro Sync reescreve e o que ele nunca toca

- Ele reescreve os arquivos `.md` acima somente enquanto estiverem exatamente como ele os escreveu, para que acompanhem as atualizações do aplicativo. Um arquivo que você modificou é deixado como está: o Astro Sync avisa e oferece “Substituir pela versão do Astro Sync”, que primeiro renomeia o seu para `<nome>.modifie-<data>.md`.
- Ele cria pastas. Nunca move, renomeia, modifica nem apaga os seus quadros, os seus registros, as suas imagens nem qualquer arquivo que você tenha copiado, movido ou vinculado aqui. Os arquivos de processamento dele (masters, resultados do empilhamento, arquivos temporários) só são apagados se você pedir, em Configurações › Geral › Arquivos de processamento.

## Nada é enviado a uma IA

O Astro Sync não contata nenhum serviço de IA. “Perguntar a um assistente de IA” e “Fazer uma pergunta a uma IA” apenas copiam um texto para a sua área de transferência: você o entrega ao assistente de sua escolha, que lê estes arquivos com o próprio acesso ao seu computador. Os quadros FITS e os registros brutos nunca saem do seu computador; só os resultados que você escolher enviar vão para o seu espaço do Astro Portal.
