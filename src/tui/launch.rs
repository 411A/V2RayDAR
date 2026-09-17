//! Detached OS helpers: spawn a viewer without blocking the TUI.
//!
//! What remains of the old config-file opener now that settings live in
//! `data.db`: only the spawn primitive, still used by the QR sheet viewer.

use std::{
    path::Path,
    process::{Command, Stdio},
};

#[cfg(target_os = "windows")]
use crate::constants::WINDOWS_CREATE_NO_WINDOW;

pub fn try_spawn(command: &str, args: &[String]) -> std::io::Result<()> {
    let mut command = Command::new(command);
    command.args(args);
    spawn_detached(&mut command)
}

fn spawn_detached(command: &mut Command) -> std::io::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
    }

    command.spawn().map(|_| ())
}

pub fn path_arg(path: &Path) -> String {
    path.display().to_string()
}
