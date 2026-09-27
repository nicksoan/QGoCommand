//! Port of `QGo.App.Services.CommandExecutionService` plus the
//! `DetermineShortcutType` classifier that both `MainViewModel` and
//! `ChainedShortcutExecutor` carry a copy of.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Port of `QGo.App.Services.CommandType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandType {
    CommandPrompt,
    PowerShell,
    PowerShellCore,
}

impl CommandType {
    /// Matches `CommandType.ToString()`, which telemetry sends as a string.
    pub fn name(self) -> &'static str {
        match self {
            Self::CommandPrompt => "CommandPrompt",
            Self::PowerShell => "PowerShell",
            Self::PowerShellCore => "PowerShellCore",
        }
    }

    /// The executable `ProcessStartInfo.FileName` is set to.
    pub fn executable(self) -> &'static str {
        match self {
            Self::CommandPrompt => "cmd.exe",
            Self::PowerShell => "powershell.exe",
            Self::PowerShellCore => "pwsh.exe",
        }
    }

    /// The `ProcessStartInfo.Arguments` string, verbatim from the C#.
    ///
    /// Each shell is left open (`/k`, `-NoExit`) and the PowerShell variants
    /// force UTF-8 output so non-ASCII command output is not mangled.
    pub fn arguments(self, command: &str) -> String {
        match self {
            Self::CommandPrompt => format!("/k {command}"),
            Self::PowerShell => format!(
                "-NoExit -Command \"chcp 65001 | Out-Null; \
                 [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {command}\""
            ),
            Self::PowerShellCore => format!(
                "-NoExit -Command \"[Console]::OutputEncoding = \
                 [System.Text.Encoding]::UTF8; {command}\""
            ),
        }
    }

    /// The `ShortcutTypeUsage` bucket used by telemetry.
    pub fn telemetry_type(self) -> &'static str {
        match self {
            Self::CommandPrompt => "cmd_command",
            Self::PowerShell => "powershell_command",
            Self::PowerShellCore => "powershell_core_command",
        }
    }
}

impl fmt::Display for CommandType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// `CommandExecutionService.DetectCommandType`.
pub fn detect_command_type(template: &str) -> Option<CommandType> {
    if template.trim().is_empty() {
        return None;
    }
    // C# lowercases first and trims afterwards; the order is irrelevant here
    // but is kept so the two implementations read the same.
    let lower = template.to_lowercase();
    let lower = lower.trim();

    // Direct command prefixes.
    if lower.starts_with("cmd:") || lower.starts_with("cmd ") {
        return Some(CommandType::CommandPrompt);
    }
    if lower.starts_with("ps:") || lower.starts_with("powershell:") {
        return Some(CommandType::PowerShell);
    }
    if lower.starts_with("pwsh:") || lower.starts_with("powershell-core:") {
        return Some(CommandType::PowerShellCore);
    }

    // File-based execution.
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        return Some(CommandType::CommandPrompt);
    }
    if lower.ends_with(".ps1") {
        return Some(CommandType::PowerShell);
    }

    None
}

/// `CommandExecutionService.ExtractCommand` — strips the scheme-like prefix and
/// returns file-based targets untouched.
pub fn extract_command(template: &str) -> String {
    if template.trim().is_empty() {
        return String::new();
    }
    let trimmed = template.trim();

    // Every prefix below is ASCII, so byte slicing is safe.
    for prefix in ["powershell-core:", "powershell:", "pwsh:", "cmd:", "ps:"] {
        if trimmed.len() >= prefix.len() && trimmed[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return trimmed[prefix.len()..].trim().to_string();
        }
    }

    trimmed.to_string()
}

/// `DetermineShortcutType` — the label recorded against every execution in
/// `usage-stats.json` (`web_url`, `local_path`, `executable`, ...).
pub fn determine_shortcut_type(template: &str) -> &'static str {
    if template.is_empty() {
        return "empty";
    }

    if let Some(kind) = detect_command_type(template) {
        return kind.telemetry_type();
    }

    let lower = template.to_lowercase();

    if lower.starts_with("http://") || lower.starts_with("https://") {
        return "web_url";
    }
    if lower.contains("://") {
        return "protocol_url";
    }
    if lower.ends_with(".exe") || lower.contains(".exe ") {
        return "executable";
    }
    // The C# writes `lower.StartsWith("%") && lower.Contains("%")`, which is
    // true for any target that merely begins with '%'.
    if lower.starts_with('%') {
        return "environment_path";
    }
    if lower.starts_with("\\\\") {
        return "network_path";
    }
    let chars: Vec<char> = lower.chars().collect();
    if chars.len() >= 3 && chars[1] == ':' && chars[2] == '\\' {
        return "local_path";
    }
    if Path::new(template).exists() {
        return "file_system";
    }

    "other"
}

/// Port of `QGo.App.Services.CommandExecutionResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandExecutionResult {
    pub success: bool,
    pub exit_code: i32,
    pub output: String,
    pub error: String,
    pub message: String,
}

impl CommandExecutionResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            success: true,
            exit_code: 0,
            output: String::new(),
            error: String::new(),
            message: message.into(),
        }
    }

    /// `CommandExecutionResult.Failure` — exit code -1 is what the view model
    /// checks before showing an error dialog.
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            exit_code: -1,
            output: String::new(),
            error: String::new(),
            message: message.into(),
        }
    }

    /// `CommandExecutionResult.Cancelled`.
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self {
            success: false,
            exit_code: -2,
            output: String::new(),
            error: String::new(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_prefixed_commands() {
        assert_eq!(
            detect_command_type("cmd: dir"),
            Some(CommandType::CommandPrompt)
        );
        assert_eq!(
            detect_command_type("cmd ipconfig"),
            Some(CommandType::CommandPrompt)
        );
        assert_eq!(
            detect_command_type("ps: Get-Process"),
            Some(CommandType::PowerShell)
        );
        assert_eq!(
            detect_command_type("PowerShell: Get-Date"),
            Some(CommandType::PowerShell)
        );
        assert_eq!(
            detect_command_type("pwsh: ls"),
            Some(CommandType::PowerShellCore)
        );
        assert_eq!(
            detect_command_type("powershell-core: ls"),
            Some(CommandType::PowerShellCore)
        );
    }

    #[test]
    fn detects_script_files_by_extension() {
        assert_eq!(
            detect_command_type("C:\\x\\go.bat"),
            Some(CommandType::CommandPrompt)
        );
        assert_eq!(
            detect_command_type("C:\\x\\go.CMD"),
            Some(CommandType::CommandPrompt)
        );
        assert_eq!(
            detect_command_type("C:\\x\\go.ps1"),
            Some(CommandType::PowerShell)
        );
    }

    #[test]
    fn plain_targets_are_not_commands() {
        assert_eq!(detect_command_type("https://github.com/"), None);
        assert_eq!(detect_command_type("notepad.exe"), None);
        assert_eq!(detect_command_type(""), None);
        assert_eq!(detect_command_type("   "), None);
    }

    #[test]
    fn extract_strips_the_prefix_but_keeps_file_paths_whole() {
        assert_eq!(extract_command("cmd: dir /w"), "dir /w");
        assert_eq!(extract_command("PS:  Get-Process "), "Get-Process");
        assert_eq!(extract_command("powershell-core: ls"), "ls");
        assert_eq!(extract_command("C:\\x\\go.ps1"), "C:\\x\\go.ps1");
    }

    #[test]
    fn extract_prefers_the_longest_matching_prefix() {
        // "powershell-core:" must win over "powershell:" and "ps:".
        assert_eq!(extract_command("powershell-core:ls"), "ls");
        assert_eq!(extract_command("powershell:ls"), "ls");
    }

    #[test]
    fn argument_strings_match_the_csharp_verbatim() {
        assert_eq!(CommandType::CommandPrompt.arguments("dir"), "/k dir");
        assert!(CommandType::PowerShell
            .arguments("Get-Date")
            .starts_with("-NoExit -Command \"chcp 65001"));
        assert!(CommandType::PowerShellCore
            .arguments("ls")
            .contains("-NoExit -Command"));
    }

    #[test]
    fn classifies_shortcut_types_for_telemetry() {
        assert_eq!(determine_shortcut_type(""), "empty");
        assert_eq!(determine_shortcut_type("https://x.com"), "web_url");
        assert_eq!(determine_shortcut_type("steam://run/440"), "protocol_url");
        assert_eq!(determine_shortcut_type("notepad.exe"), "executable");
        assert_eq!(determine_shortcut_type("%HOMEPATH%"), "environment_path");
        assert_eq!(determine_shortcut_type("\\\\server\\share"), "network_path");
        assert_eq!(determine_shortcut_type("C:\\Windows"), "local_path");
        assert_eq!(determine_shortcut_type("cmd: dir"), "cmd_command");
        assert_eq!(determine_shortcut_type("ps: ls"), "powershell_command");
        assert_eq!(determine_shortcut_type("something odd"), "other");
    }

    #[test]
    fn result_exit_codes_match_what_the_view_model_switches_on() {
        assert_eq!(CommandExecutionResult::failure("x").exit_code, -1);
        assert_eq!(CommandExecutionResult::cancelled("x").exit_code, -2);
        assert!(CommandExecutionResult::success("x").success);
    }
}
