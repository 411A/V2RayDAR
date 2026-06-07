use std::process::Command;

use anyhow::{Result, anyhow};

use crate::constants::FIREWALL_RULE_NAME;

pub fn apply(enabled: bool, port: u16) -> Result<String> {
    if cfg!(target_os = "windows") {
        windows(enabled, port)
    } else if cfg!(target_os = "linux") {
        linux(enabled, port)
    } else {
        Ok("Firewall auto-change is unsupported on this OS".to_string())
    }
}

fn windows(enabled: bool, port: u16) -> Result<String> {
    if enabled {
        run(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={FIREWALL_RULE_NAME}"),
                "dir=in",
                "action=allow",
                "protocol=TCP",
                &format!("localport={port}"),
            ],
        )?;
        Ok(format!(
            "Sharing enabled; Windows firewall allows TCP {port}"
        ))
    } else {
        run(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                &format!("name={FIREWALL_RULE_NAME}"),
            ],
        )?;
        Ok("Sharing disabled; Windows firewall rule removed".to_string())
    }
}

fn linux(enabled: bool, port: u16) -> Result<String> {
    if command_exists("ufw") {
        let action = if enabled { "allow" } else { "delete" };
        run("ufw", &[action, &format!("{port}/tcp")])?;
        return Ok(format!(
            "Sharing {}; ufw updated for TCP {port}",
            on_off(enabled)
        ));
    }

    if command_exists("firewall-cmd") {
        let action = if enabled {
            "--add-port"
        } else {
            "--remove-port"
        };
        run(
            "firewall-cmd",
            &[action, &format!("{port}/tcp"), "--permanent"],
        )?;
        let _ = run("firewall-cmd", &["--reload"]);
        return Ok(format!(
            "Sharing {}; firewalld updated for TCP {port}",
            on_off(enabled)
        ));
    }

    Ok("Sharing changed; no supported Linux firewall tool found".to_string())
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
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

fn on_off(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "disabled" }
}
