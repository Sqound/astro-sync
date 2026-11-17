//! « Exporter pour partager »: where the copy of an image with its band is
//! saved. The band itself is drawn by the window, on a copy made on request
//! (`src/lib/shareBand*.ts`); these two commands only ask for the place and
//! write the bytes through the core's guarded writer
//! (`astro_sync_core::share_copy`), which strips the metadata and never lets
//! the copy replace the image it comes from.

use std::path::Path;

use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

use astro_sync_core::share_copy;

use crate::commands::run_blocking;
use crate::dialogs;
use crate::error::CommandError;

/// The place of the copy, asked before anything is drawn. `None`: the
/// dialog was dismissed.
#[tauri::command]
pub async fn share_copy_choose<R: Runtime>(
    app: AppHandle<R>,
    file_name: String,
) -> Result<Option<String>, CommandError> {
    dialogs::pick(|answer| {
        app.dialog()
            .file()
            .add_filter("PNG", &["png"])
            .set_file_name(&file_name)
            .save_file(answer)
    })
    .await
}

fn header(request: &Request<'_>, name: &str) -> Result<String, CommandError> {
    let value = request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| CommandError::other(format!("missing {name}")))?;
    // The window sends paths percent-encoded: a header holds ASCII only.
    let mut bytes = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut at = 0;
    while at < raw.len() {
        if raw[at] == b'%' && at + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[at + 1..at + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                bytes.push(byte);
                at += 3;
                continue;
            }
        }
        bytes.push(raw[at]);
        at += 1;
    }
    String::from_utf8(bytes).map_err(|error| CommandError::other(error.to_string()))
}

/// Writes the copy (the request's raw body, a PNG) at the `path` header,
/// refusing the image named by the `source` header.
#[tauri::command]
pub async fn share_copy_write(request: Request<'_>) -> Result<(), CommandError> {
    let path = header(&request, "x-share-path")?;
    let source = header(&request, "x-share-source")?;
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(CommandError::other("the copy was not sent as bytes"));
    };
    let bytes = bytes.clone();
    run_blocking(move || {
        share_copy::write_share_copy(Path::new(&path), Path::new(&source), &bytes)
            .map_err(|error| CommandError::new(error.code(), error.to_string()))
    })
    .await
}
