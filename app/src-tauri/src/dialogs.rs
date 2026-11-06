//! The system's file and folder pickers (`tauri-plugin-dialog`), asked
//! without ever holding the thread that received the request.
//!
//! Changing the default folder in the settings froze the application. On macOS a plain `fn` command
//! runs inline on the main thread, and `blocking_pick_folder` /
//! `blocking_pick_file` wait there, on a semaphore, for the dialog to answer
//! -- but the dialog is opened by the main thread's own event loop, which is
//! the thread that is waiting: a deadlock, with no way out but a forced quit
//! (the `sample` of the frozen process shows the main thread in
//! `WebURLSchemeHandlerCocoa::platformStartTask` -> Rust ->
//! `semaphore_wait_trap`). The rule of the house (`commands.rs`): nothing blocking on the thread
//! that receives the request.
//!
//! So a picker command is an `async fn`, and it uses the plugin's callback
//! API (`pick_folder(|path| ..)`, `pick_file(|path| ..)`): the dialog is
//! opened on the main thread by the plugin, the callback fires when the
//! person has answered, and the command's future simply waits for that
//! answer without holding any thread, the main thread least of all.

use tauri::async_runtime::channel;
use tauri_plugin_dialog::FilePath;

use crate::error::CommandError;

/// What a picker's callback receives: the chosen path, `None` when the
/// person dismissed the dialog.
pub(crate) type PickerCallback = Box<dyn FnOnce(Option<FilePath>) + Send + 'static>;

/// Opens a picker with `open` (which must hand the callback to the plugin
/// and return at once, as `FileDialogBuilder::pick_folder` / `pick_file`
/// do), then waits, asynchronously, for the answer. `Ok(None)`: dismissed,
/// not an error.
pub(crate) async fn pick<F>(open: F) -> Result<Option<String>, CommandError>
where
    F: FnOnce(PickerCallback),
{
    let (sender, mut receiver) = channel::<Option<FilePath>>(1);
    open(Box::new(move |picked| {
        // Capacity 1 and one answer: this never has to wait, so it is safe
        // from whichever thread the plugin calls back on.
        let _ = sender.try_send(picked);
    }));
    match receiver.recv().await {
        Some(Some(file_path)) => {
            let path = file_path
                .into_path()
                .map_err(|error| CommandError::other(error.to_string()))?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
        Some(None) => Ok(None),
        // The callback was dropped without an answer (the application is
        // closing, or the dialog could not be opened): say so in words.
        None => Err(CommandError::other(
            "the system dialog closed without an answer".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    //! The deadlock cannot be reproduced without a window, so the tests pin
    //! the two things it depends on: the picker commands are `async` (a
    //! plain `fn` command runs on the main thread), and `pick` hands the
    //! thread back at once while the answer can only come from the very
    //! thread that asked (the main thread's event loop, simulated here by
    //! the test thread firing the callback itself).

    use super::*;
    use std::cell::RefCell;
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::pin;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;
    use tauri::test::MockRuntime;
    use tauri::AppHandle;

    /// Compiles only for a function that returns a future: the day
    /// `choose_folder` or `fits_choose` is a plain `fn` again, this file
    /// stops compiling.
    fn is_async<F: Future>(_command: fn(AppHandle<MockRuntime>) -> F) {}
    /// The same for a picker that takes its starting folder.
    fn is_async_from<F: Future>(_command: fn(AppHandle<MockRuntime>, Option<String>) -> F) {}

    #[test]
    fn the_picker_commands_are_async() {
        is_async_from(crate::commands::choose_folder::<MockRuntime>);
        is_async(crate::commands_targets::fits_choose::<MockRuntime>);
    }

    #[test]
    fn the_command_source_never_waits_for_a_dialog() {
        // The plugin's `blocking_*` calls are the deadlock itself. They must
        // not come back in a command file (this file's comments name them).
        for source in [
            include_str!("commands.rs"),
            include_str!("commands_night.rs"),
            include_str!("commands_targets.rs"),
        ] {
            for line in source.lines() {
                let code = line.split("//").next().unwrap();
                assert!(
                    !code.contains("blocking_pick"),
                    "a command waits for a dialog on the thread that received it: {line}"
                );
            }
        }
    }

    #[test]
    fn asking_for_a_folder_gives_the_thread_back_and_the_answer_comes_from_that_thread() {
        // The "plugin": it keeps the callback, and only this thread (the
        // "main thread") may fire it.
        let slot: Rc<RefCell<Option<PickerCallback>>> = Rc::default();
        let kept = Rc::clone(&slot);
        let mut ask = pin!(pick(move |callback| {
            *kept.borrow_mut() = Some(callback);
        }));
        let mut context = Context::from_waker(Waker::noop());

        // Asking returns at once: the thread is free to run the event loop.
        assert!(matches!(ask.as_mut().poll(&mut context), Poll::Pending));
        let callback = slot.borrow_mut().take().expect("the dialog was asked");

        // The event loop answers (the person chose a folder).
        callback(Some(FilePath::from(PathBuf::from("chosen-folder"))));
        match ask.as_mut().poll(&mut context) {
            Poll::Ready(Ok(Some(path))) => assert_eq!(path, "chosen-folder"),
            other => panic!("expected the chosen path, got {other:?}"),
        }
    }

    #[test]
    fn dismissing_the_dialog_is_not_an_error() {
        let slot: Rc<RefCell<Option<PickerCallback>>> = Rc::default();
        let kept = Rc::clone(&slot);
        let mut ask = pin!(pick(move |callback| {
            *kept.borrow_mut() = Some(callback);
        }));
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(ask.as_mut().poll(&mut context), Poll::Pending));
        (slot.borrow_mut().take().unwrap())(None);
        assert!(matches!(
            ask.as_mut().poll(&mut context),
            Poll::Ready(Ok(None))
        ));
    }

    #[test]
    fn a_dialog_that_never_answers_is_an_error_in_words_not_a_hang() {
        let mut ask = pin!(pick(drop));
        let mut context = Context::from_waker(Waker::noop());
        match ask.as_mut().poll(&mut context) {
            Poll::Ready(Err(error)) => assert!(error.message.contains("without an answer")),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn an_answer_from_another_thread_wakes_the_waiting_command() {
        let answer = tauri::async_runtime::block_on(pick(|callback| {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                callback(Some(FilePath::from(PathBuf::from("later"))));
            });
        }));
        assert_eq!(answer.unwrap(), Some("later".to_string()));
    }
}
