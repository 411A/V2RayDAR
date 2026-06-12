use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(target_os = "windows")]
use crate::constants::WINDOWS_CREATE_NO_WINDOW;

pub fn open(path: &Path) -> String {
    if let Some(message) = open_vscode(path) {
        return message;
    }

    if cfg!(target_os = "windows") {
        return open_windows(path);
    }

    if cfg!(target_os = "macos") {
        return open_macos(path);
    }

    open_linux(path)
}

fn open_vscode(path: &Path) -> Option<String> {
    if cfg!(target_os = "windows") {
        return open_windows_vscode(path);
    }

    if cfg!(target_os = "macos") {
        if try_spawn("code", &[path_arg(path)]).is_ok()
            || try_spawn(
                "open",
                &[
                    "-a".to_string(),
                    "Visual Studio Code".to_string(),
                    path_arg(path),
                ],
            )
            .is_ok()
        {
            return Some("Opening config with VS Code".to_string());
        }
        return None;
    }

    if try_spawn("code", &[path_arg(path)]).is_ok() {
        return Some("Opening config with VS Code".to_string());
    }

    None
}

fn open_windows_vscode(path: &Path) -> Option<String> {
    for candidate in windows_vscode_candidates() {
        if !candidate.exists() {
            continue;
        }

        if candidate
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("cmd"))
        {
            continue;
        }

        if try_spawn_path(&candidate, &[path_arg(path)]).is_ok() {
            return Some("Opening config with VS Code".to_string());
        }
    }

    None
}

fn windows_vscode_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(command_candidates("code", &["cmd", "exe"]));
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let root = PathBuf::from(local_app_data).join("Programs");
        candidates.push(root.join("Microsoft VS Code").join("Code.exe"));
        candidates.push(root.join("Microsoft VS Code").join("bin").join("code.cmd"));
        candidates.push(
            root.join("Microsoft VS Code Insiders")
                .join("Code - Insiders.exe"),
        );
        candidates.push(
            root.join("Microsoft VS Code Insiders")
                .join("bin")
                .join("code-insiders.cmd"),
        );
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        let root = PathBuf::from(program_files);
        candidates.push(root.join("Microsoft VS Code").join("Code.exe"));
        candidates.push(root.join("Microsoft VS Code").join("bin").join("code.cmd"));
    }
    if let Some(program_files_x86) = env::var_os("ProgramFiles(x86)") {
        let root = PathBuf::from(program_files_x86);
        candidates.push(root.join("Microsoft VS Code").join("Code.exe"));
        candidates.push(root.join("Microsoft VS Code").join("bin").join("code.cmd"));
    }
    candidates
}

fn command_candidates(command: &str, extensions: &[&str]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(paths) = env::var_os("PATH") else {
        return candidates;
    };

    for path in env::split_paths(&paths) {
        candidates.push(path.join(command));
        for extension in extensions {
            candidates.push(path.join(format!("{command}.{extension}")));
        }
    }

    candidates
}

fn open_windows(path: &Path) -> String {
    if try_spawn("notepad", &[path_arg(path)]).is_ok() {
        return "Opening config with Notepad".to_string();
    }

    if try_spawn("explorer", &[format!("/select,{}", path.display())]).is_ok() {
        return "Revealing config in File Explorer".to_string();
    }

    format!("Edit config manually: {}", path.display())
}

fn open_macos(path: &Path) -> String {
    if try_spawn("open", &[path_arg(path)]).is_ok() {
        return "Opening config with the default macOS editor".to_string();
    }

    if let Some(parent) = path.parent()
        && try_spawn("open", &[path_arg(parent)]).is_ok()
    {
        return "Revealing config folder in Finder".to_string();
    }

    format!("Edit config manually: {}", path.display())
}

fn open_linux(path: &Path) -> String {
    if command_available("nano") && try_spawn_terminal("nano", path).is_ok() {
        return "Opening config with nano".to_string();
    }

    if try_spawn("xdg-open", &[path_arg(path)]).is_ok() {
        return "Opening config with the default editor".to_string();
    }

    if let Some(parent) = path.parent()
        && try_spawn("xdg-open", &[path_arg(parent)]).is_ok()
    {
        return "Opening config folder".to_string();
    }

    format!("Edit config manually: {}", path.display())
}

fn try_spawn(command: &str, args: &[String]) -> std::io::Result<()> {
    let mut command = Command::new(command);
    command.args(args);
    spawn_detached(&mut command)
}

fn try_spawn_path(command: &Path, args: &[String]) -> std::io::Result<()> {
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

fn try_spawn_terminal(editor: &str, path: &Path) -> std::io::Result<()> {
    for terminal in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
        let result = match terminal {
            "gnome-terminal" => Command::new(terminal)
                .args(["--", editor, &path_arg(path)])
                .spawn(),
            _ => Command::new(terminal)
                .args(["-e", editor, &path_arg(path)])
                .spawn(),
        };

        if let Ok(mut child) = result {
            let _ = child.try_wait();
            return Ok(());
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no supported terminal emulator found",
    ))
}

fn command_available(command: &str) -> bool {
    if cfg!(target_os = "windows") {
        return command_candidates(command, &["exe", "cmd", "bat"])
            .into_iter()
            .any(|path| path.exists());
    }

    Command::new(command)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn path_arg(path: &Path) -> String {
    path.display().to_string()
}
