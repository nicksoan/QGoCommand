//! Classification and persistence for executable shortcut trust decisions.

use crate::command::{detect_command_type, determine_shortcut_type, CommandType};
use crate::paths;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskKind {
    CommandPrompt,
    PowerShell,
    PowerShellCore,
    Executable,
}

impl RiskKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::CommandPrompt => "Command Prompt command",
            Self::PowerShell => "Windows PowerShell command",
            Self::PowerShellCore => "PowerShell command",
            Self::Executable => "Application",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRisk {
    pub kind: RiskKind,
    pub shortcut_key: String,
    pub target: String,
    pub fingerprint: String,
}

impl LaunchRisk {
    pub fn new(kind: RiskKind, shortcut_key: impl Into<String>, target: impl Into<String>) -> Self {
        let shortcut_key = shortcut_key.into();
        let target = target.into();
        let mut hasher = Sha256::new();
        hasher.update(b"qgo-launch-trust-v1\0");
        hasher.update(target.as_bytes());
        let fingerprint = format!("{:x}", hasher.finalize());
        Self {
            kind,
            shortcut_key,
            target,
            fingerprint,
        }
    }
}

pub fn classify_risk(target: &str) -> Option<RiskKind> {
    if let Some(command) = detect_command_type(target) {
        return Some(match command {
            CommandType::CommandPrompt => RiskKind::CommandPrompt,
            CommandType::PowerShell => RiskKind::PowerShell,
            CommandType::PowerShellCore => RiskKind::PowerShellCore,
        });
    }

    (determine_shortcut_type(target) == "executable").then_some(RiskKind::Executable)
}

pub fn display_classification(target: &str) -> &'static str {
    match determine_shortcut_type(target) {
        "cmd_command" => "Command Prompt - runs code",
        "powershell_command" => "Windows PowerShell - runs code",
        "powershell_core_command" => "PowerShell - runs code",
        "executable" => "Application - runs code",
        "web_url" => "Website",
        "protocol_url" => "External protocol",
        "network_path" => "Network location",
        "local_path" | "environment_path" | "file_system" => "Local file or folder",
        "empty" => "Empty target",
        _ => "Other target",
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TrustedLaunchFile {
    #[serde(default)]
    fingerprints: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TrustedLaunches {
    fingerprints: HashSet<String>,
}

impl TrustedLaunches {
    pub fn load() -> Self {
        let Ok(text) = fs::read_to_string(paths::trusted_launches_path()) else {
            return Self::default();
        };
        match serde_json::from_str::<TrustedLaunchFile>(&text) {
            Ok(file) => Self {
                fingerprints: file.fingerprints.into_iter().collect(),
            },
            Err(error) => {
                log::warn!("trusted-launches.json could not be parsed: {error}");
                Self::default()
            }
        }
    }

    pub fn contains(&self, risk: &LaunchRisk) -> bool {
        self.fingerprints.contains(&risk.fingerprint)
    }

    pub fn trust_all(&mut self, risks: &[LaunchRisk]) -> std::io::Result<()> {
        let mut updated = self.fingerprints.clone();
        updated.extend(risks.iter().map(|risk| risk.fingerprint.clone()));
        Self::save_fingerprints(&updated)?;
        self.fingerprints = updated;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.fingerprints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fingerprints.is_empty()
    }

    pub fn clear_saved() -> std::io::Result<()> {
        Self::save_fingerprints(&HashSet::new())
    }

    fn save_fingerprints(fingerprints: &HashSet<String>) -> std::io::Result<()> {
        if let Some(parent) = paths::trusted_launches_path().parent() {
            fs::create_dir_all(parent)?;
        }
        let mut fingerprints: Vec<String> = fingerprints.iter().cloned().collect();
        fingerprints.sort();
        let json = serde_json::to_string_pretty(&TrustedLaunchFile { fingerprints })
            .map_err(std::io::Error::other)?;
        let path = paths::trusted_launches_path();
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, json)?;
        fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_scripts_and_executables_are_risky() {
        assert_eq!(
            classify_risk("cmd: echo hello"),
            Some(RiskKind::CommandPrompt)
        );
        assert_eq!(classify_risk("script.ps1"), Some(RiskKind::PowerShell));
        assert_eq!(
            classify_risk(r"C:\Tools\thing.exe --quiet"),
            Some(RiskKind::Executable)
        );
        assert_eq!(classify_risk("https://example.com"), None);
        assert_eq!(classify_risk(r"C:\Documents\report.pdf"), None);
    }

    #[test]
    fn trust_fingerprint_changes_with_executable_content() {
        let first = LaunchRisk::new(RiskKind::PowerShell, "test", "ps: Write-Host one");
        let changed = LaunchRisk::new(RiskKind::PowerShell, "test", "ps: Write-Host two");
        assert_ne!(first.fingerprint, changed.fingerprint);
    }

    #[test]
    fn shortcut_name_does_not_change_exact_target_trust() {
        let first = LaunchRisk::new(RiskKind::Executable, "one", "tool.exe");
        let renamed = LaunchRisk::new(RiskKind::Executable, "two", "tool.exe");
        assert_eq!(first.fingerprint, renamed.fingerprint);
    }
}
