use std::{fs, path::Path, process::Command};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::constants::{APP_NAME, FIREWALL_RULE_NAME, FIREWALL_STATE_FILE_NAME};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct FirewallState {
    #[serde(default)]
    app: String,
    rules: Vec<OwnedFirewallRule>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct OwnedFirewallRule {
    backend: FirewallBackend,
    port: u16,
    #[serde(default = "default_rule_name")]
    rule_name: String,
}

fn default_rule_name() -> String {
    FIREWALL_RULE_NAME.to_string()
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FirewallBackend {
    WindowsNetsh,
    LinuxUfw,
    LinuxFirewalld,
}

pub fn apply(state_dir: &Path, enabled: bool, port: u16, rule_name: &str) -> Result<String> {
    let state_path = firewall_state_path(state_dir);
    if cfg!(target_os = "windows") {
        read_state(&state_path)?;
        windows(enabled, port, rule_name)?;
        if enabled {
            record_rule(&state_path, FirewallBackend::WindowsNetsh, port, rule_name)?;
            Ok(format!("Windows firewall allows TCP {port} ({rule_name})"))
        } else {
            remove_recorded_rule_by_port(&state_path, FirewallBackend::WindowsNetsh, port)?;
            Ok(format!("Windows firewall rule removed for TCP {port}"))
        }
    } else if cfg!(target_os = "linux") {
        linux(&state_path, enabled, port, rule_name)
    } else {
        Ok("Firewall auto-change is unsupported on this OS".to_string())
    }
}

pub fn remove_owned_rules(state_dir: &Path) -> Result<Vec<String>> {
    let mut messages = Vec::new();
    let mut failures = Vec::new();
    let state_path = firewall_state_path(state_dir);
    let mut state = read_state(&state_path)?;

    let mut remaining_rules = Vec::new();
    for rule in &state.rules {
        match remove_owned_rule(rule) {
            Ok(message) => messages.push(message),
            Err(error) => {
                failures.push(format!("firewall rule was not removed: {error}"));
                remaining_rules.push(rule.clone());
            }
        }
    }
    state.rules = remaining_rules;
    write_state(&state_path, &state)?;

    if !failures.is_empty() {
        return Err(anyhow!(
            "unable to remove all V2RayDAR-owned firewall rules: {}",
            failures.join("; ")
        ));
    }

    Ok(messages)
}

fn firewall_state_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join(FIREWALL_STATE_FILE_NAME)
}

fn linux(state_path: &Path, enabled: bool, port: u16, rule_name: &str) -> Result<String> {
    read_state(state_path)?;

    if command_exists("ufw") {
        if enabled {
            let existed = ufw_allows_port(port)?;
            run("ufw", &["allow", &format!("{port}/tcp")])?;
            if !existed {
                record_rule(state_path, FirewallBackend::LinuxUfw, port, rule_name)?;
            }
            let ownership = if existed {
                "pre-existing rule left user-owned"
            } else {
                "owned rule recorded"
            };
            return Ok(format!("{rule_name}: ufw allows TCP {port}; {ownership}"));
        }
        return if remove_recorded_rule_by_port(state_path, FirewallBackend::LinuxUfw, port)? {
            Ok(format!("{rule_name}: removed owned ufw TCP {port} rule"))
        } else {
            Ok(format!(
                "{rule_name}: no owned ufw TCP {port} rule was recorded"
            ))
        };
    }

    if command_exists("firewall-cmd") {
        if enabled {
            let existed = firewalld_allows_port(port)?;
            firewalld_port("--add-port", port)?;
            if !existed {
                record_rule(state_path, FirewallBackend::LinuxFirewalld, port, rule_name)?;
            }
            let ownership = if existed {
                "pre-existing rule left user-owned"
            } else {
                "owned rule recorded"
            };
            return Ok(format!(
                "{rule_name}: firewalld allows TCP {port}; {ownership}"
            ));
        }
        return if remove_recorded_rule_by_port(state_path, FirewallBackend::LinuxFirewalld, port)? {
            Ok(format!(
                "{rule_name}: removed owned firewalld TCP {port} rule"
            ))
        } else {
            Ok(format!(
                "{rule_name}: no owned firewalld TCP {port} rule was recorded"
            ))
        };
    }

    Ok("Firewall changed; no supported Linux firewall tool found".to_string())
}

fn windows(enabled: bool, port: u16, rule_name: &str) -> Result<()> {
    if enabled {
        run(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={rule_name}"),
                "dir=in",
                "action=allow",
                "protocol=TCP",
                &format!("localport={port}"),
            ],
        )
    } else {
        remove_windows_rule(rule_name)
    }
}

fn remove_windows_rule(rule_name: &str) -> Result<()> {
    let output = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={rule_name}"),
        ])
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");
    if combined.contains("No rules") || combined.contains("does not exist") {
        return Ok(());
    }
    Err(anyhow!(
        "firewall command failed; run as admin/root if needed"
    ))
}

fn remove_owned_rule(rule: &OwnedFirewallRule) -> Result<String> {
    match rule.backend {
        FirewallBackend::WindowsNetsh => {
            remove_windows_rule(&rule.rule_name)?;
            Ok(format!(
                "removed Windows firewall rule '{}' for TCP {}",
                rule.rule_name, rule.port
            ))
        }
        FirewallBackend::LinuxUfw => {
            run("ufw", &["delete", "allow", &format!("{}/tcp", rule.port)])?;
            Ok(format!("removed ufw TCP {} rule", rule.port))
        }
        FirewallBackend::LinuxFirewalld => {
            firewalld_port("--remove-port", rule.port)?;
            Ok(format!("removed firewalld TCP {} rule", rule.port))
        }
    }
}

fn firewalld_port(action: &str, port: u16) -> Result<()> {
    run(
        "firewall-cmd",
        &[action, &format!("{port}/tcp"), "--permanent"],
    )?;
    let _ = run("firewall-cmd", &["--reload"]);
    Ok(())
}

fn record_rule(
    state_path: &Path,
    backend: FirewallBackend,
    port: u16,
    rule_name: &str,
) -> Result<()> {
    let mut state = read_state(state_path)?;
    if !state
        .rules
        .iter()
        .any(|rule| rule.backend == backend && rule.port == port)
    {
        state.rules.push(OwnedFirewallRule {
            backend,
            port,
            rule_name: rule_name.to_string(),
        });
    }
    write_state(state_path, &state)
}

fn remove_recorded_rule_by_port(
    state_path: &Path,
    backend: FirewallBackend,
    port: u16,
) -> Result<bool> {
    let mut state = read_state(state_path)?;
    let rule = state
        .rules
        .iter()
        .find(|rule| rule.backend == backend && rule.port == port)
        .cloned();
    let Some(rule) = rule else {
        return Ok(false);
    };

    remove_owned_rule(&rule)?;
    state
        .rules
        .retain(|r| !(r.backend == backend && r.port == port));
    write_state(state_path, &state)?;
    Ok(true)
}

fn read_state(path: &Path) -> Result<FirewallState> {
    match fs::read(path) {
        Ok(bytes) => {
            let state: FirewallState = serde_json::from_slice(&bytes)
                .with_context(|| format!("unable to parse {}", path.display()))?;
            if state.app != APP_NAME {
                return Err(anyhow!(
                    "refusing to use {}; it is not marked as V2RayDAR-owned",
                    path.display()
                ));
            }
            Ok(state)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(empty_state()),
        Err(error) => Err(error.into()),
    }
}

fn write_state(path: &Path, state: &FirewallState) -> Result<()> {
    if state.rules.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("unable to create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

fn empty_state() -> FirewallState {
    FirewallState {
        app: APP_NAME.to_string(),
        rules: Vec::new(),
    }
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn ufw_allows_port(port: u16) -> Result<bool> {
    let output = Command::new("ufw").arg("status").output()?;
    if !output.status.success() {
        return Err(anyhow!("unable to inspect ufw status before changing it"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let port_tcp = format!("{port}/tcp");
    Ok(stdout.lines().any(|line| {
        let mut columns = line.split_whitespace();
        columns.next() == Some(port_tcp.as_str())
            && columns.any(|column| column.eq_ignore_ascii_case("allow"))
    }))
}

fn firewalld_allows_port(port: u16) -> Result<bool> {
    let output = Command::new("firewall-cmd")
        .args(["--permanent", &format!("--query-port={port}/tcp")])
        .output()?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(anyhow!(
            "unable to inspect firewalld status before changing it"
        )),
    }
}

fn run(command: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(command).args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "firewall command failed; run as admin/root if needed"
        ))
    }
}
