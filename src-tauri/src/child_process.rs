//! Shared setup for child processes that should never surface a separate console window.

use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Prevent console applications launched by the GUI app from opening their own window.
/// Their stdout and stderr remain available to the caller when redirected.
pub(crate) fn hide_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    {
        let _ = command;
    }
}
