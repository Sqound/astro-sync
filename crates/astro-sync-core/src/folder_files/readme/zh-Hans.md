# Astro Sync：你的工作文件夹

这是 Astro Sync 的工作文件夹。它按 Astro Sync 能够读取的结构保存你每一夜的天文摄影数据。其中的一切都是普通文件或文件夹：你可以自己打开、移动、编辑或删除。

## 里面有什么

- 每一夜一个文件夹，名称为“日期 · 计划或地点 · 简短标识”，例如 `2026-10-07 · Col des Étoiles · 7f3a2c`。你可以重命名它：一个隐藏文件保存着它的身份信息。
- 一夜的文件夹内：
  - `logs/`：当晚的 NINA、PHD2 和 ASIAIR 日志；
  - `calibration/`：`darks/`、`flats/`、`dark-flats/` 和 `biases/`；
  - 每个目标一个文件夹（`M31`、`NGC 7000` 等），其中 `lights/` 存放原始 FITS 帧，`final/` 存放你处理好的图像（TIFF、JPEG 或 PNG）；
  - `stacked/` 和 `progression/`：只有在 Astro Sync 自己生成图像时才会创建。
- 名称以点开头的文件和文件夹（`.astro-sync-night.json`、`.astro-sync-target.json`、`.astro-sync-work/`）属于 Astro Sync：它们保存一夜或一个目标的身份信息，以及正在进行的任务的临时工作内容。请保持原样。

## 根目录中的 .md 文件

- `README.md`：本文件。
- `ASSISTANT-IA.md`：AI 助手整理一夜文件时遵循的步骤。无论你使用什么语言，它都是英文的。
- `GUIDE-ASTRO-SYNC.<语言>.md`（例如 `GUIDE-ASTRO-SYNC.zh-Hans.md`）：Astro Sync“指南”页面的文字，在你于指南中使用“向 AI 提问”时写入。每个你请求过的语言各有一个文件。
- `<名称>.modifie-<日期>.md`（例如 `ASSISTANT-IA.modifie-2026-10-17.md`）：你修改过的上述文件的副本，在你选择“替换为 Astro Sync 的版本”时保留下来。

## Astro Sync 会改写什么，又绝不会碰什么

- 只有当上述 .md 文件与 Astro Sync 写入时完全一致，它才会改写它们，使其跟随应用的更新。你修改过的文件会保持原样：Astro Sync 会告知你，并提供“替换为 Astro Sync 的版本”，这会先把你的文件重命名为 `<名称>.modifie-<日期>.md`。
- 它会创建文件夹。它绝不会移动、重命名、修改或删除你的帧、日志、图像，也不会处理你复制、移动或链接到这里的任何文件。它自己的处理文件（主帧、叠加结果、临时文件）只有在你要求时才会删除，位置在“设置 › 常规 › 处理文件”。

## 不会向 AI 发送任何内容

Astro Sync 不会联系任何 AI 服务。“询问 AI 助手”和“向 AI 提问”只是把一段文字复制到你的剪贴板：由你把它交给自己选择的助手，助手再通过它自己对你电脑的访问权限读取这些文件。FITS 帧和原始日志绝不会离开你的电脑；只有你选择发送的结果才会进入你的 Astro Portal 空间。
