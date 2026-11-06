# Astro Sync：你的工作資料夾

這是 Astro Sync 的工作資料夾。它依 Astro Sync 能夠讀取的結構保存你每一夜的天文攝影資料。其中的一切都是一般檔案或資料夾：你可以自行開啟、移動、編輯或刪除。

## 裡面有什麼

- 每一夜一個資料夾，名稱為「日期 · 計畫或地點 · 簡短識別碼」，例如 `2026-10-07 · Col des Étoiles · 7f3a2c`。你可以重新命名：一個隱藏檔案保存著它的身分資訊。
- 一夜的資料夾內：
  - `logs/`：當晚的 NINA、PHD2 和 ASIAIR 日誌；
  - `calibration/`：`darks/`、`flats/`、`dark-flats/` 和 `biases/`；
  - 每個目標一個資料夾（`M31`、`NGC 7000` 等），其中 `lights/` 存放原始 FITS 影格，`final/` 存放你處理好的影像（TIFF、JPEG 或 PNG）；
  - `stacked/` 和 `progression/`：只有在 Astro Sync 自己產生影像時才會建立。
- 名稱以點開頭的檔案和資料夾（`.astro-sync-night.json`、`.astro-sync-target.json`、`.astro-sync-work/`）屬於 Astro Sync：它們保存一夜或一個目標的身分資訊，以及進行中的工作的暫存內容。請保持原樣。

## 根目錄中的 .md 檔案

- `README.md`：本檔案。
- `ASSISTANT-IA.md`：AI 助理整理一夜檔案時遵循的步驟。無論你使用什麼語言，它都是英文的。
- `GUIDE-ASTRO-SYNC.<語言>.md`（例如 `GUIDE-ASTRO-SYNC.zh-Hant.md`）：Astro Sync「指南」頁面的文字，在你於指南中使用「向 AI 提問」時寫入。每個你要求過的語言各有一個檔案。
- `<名稱>.modifie-<日期>.md`（例如 `ASSISTANT-IA.modifie-2026-10-17.md`）：你修改過的上述檔案的副本，在你選擇「取代為 Astro Sync 的版本」時保留下來。

## Astro Sync 會改寫什麼，又絕不會碰什麼

- 只有當上述 .md 檔案與 Astro Sync 寫入時完全一致，它才會改寫它們，使其跟隨應用程式的更新。你修改過的檔案會保持原樣：Astro Sync 會告知你，並提供「取代為 Astro Sync 的版本」，這會先把你的檔案重新命名為 `<名稱>.modifie-<日期>.md`。
- 它會建立資料夾。它絕不會移動、重新命名、修改或刪除你的影格、日誌、影像，也不會動到你複製、移動或連結到這裡的任何檔案。它自己的處理檔案（主影格、疊圖結果、暫存檔案）只有在你要求時才會刪除，位置在「設定 › 一般 › 處理檔案」。

## 不會向 AI 傳送任何內容

Astro Sync 不會聯繫任何 AI 服務。「詢問 AI 助理」和「向 AI 提問」只是把一段文字複製到你的剪貼簿：由你把它交給自己選擇的助理，助理再透過它自己對你電腦的存取權限讀取這些檔案。FITS 影格和原始日誌絕不會離開你的電腦；只有你選擇傳送的結果才會進入你的 Astro Portal 空間。
