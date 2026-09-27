//! Deciding what to do with an expanded target, and doing it.
//!
//! The WPF app launches everything through
//! `Process.Start(new ProcessStartInfo(x) { UseShellExecute = true })`, which is
//! `ShellExecuteW` — *not* `CreateProcess`. That distinction matters: it is what
//! lets a URL, a `.pdf`, or a bare document path open with its registered
//! handler. `std::process::Command` cannot do that, so the actual launch goes
//! through `ShellExecuteW` too.
//!
//! Two call sites in the C# make slightly different decisions, and both are
//! preserved: `MainViewModel.ExecuteRegularShortcut` falls back to
//! `explorer /select,"target"`, while `ChainedShortcutExecutor.LaunchTarget`
//! falls back to shell-executing the target and additionally passes the raw
//! (un-encoded) parameter as arguments when the target is an `.exe`.
//!
//! [`plan_regular`] and [`plan_chain_item`] are pure so the routing is testable
//! without spawning anything; [`execute_plan`] is the Windows-only half.

use crate::command::{detect_command_type, extract_command, CommandType};
use std::path::Path;

/// What the launcher decided to do with a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// Open a shell window and leave it open.
    Command { kind: CommandType, command: String },
    /// `ShellExecuteW(open, file, params)`.
    Shell {
        file: String,
        params: Option<String>,
    },
    /// `explorer.exe` with a pre-built argument string.
    Explorer { args: String },
    /// Empty target — the C# logs and returns without launching.
    Nothing,
}

/// True for `http:`, `mailto:`, `steam:` and friends.
///
/// A single-letter prefix is a DOS drive (`C:\...`), never a URI scheme — .NET's
/// `Uri` class treats those as `file:` URIs, and so does the routing below by
/// falling through to the path branches.
pub fn has_uri_scheme(target: &str) -> bool {
    let Some(colon) = target.find(':') else {
        return false;
    };
    if colon < 2 {
        return false;
    }
    let scheme = &target[..colon];
    let mut chars = scheme.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

/// True for `\\server\share`.
///
/// The C# checks this before touching the filesystem on purpose: `File.Exists`
/// against an unreachable share blocks the calling thread, so UNC targets are
/// handed straight to Explorer.
fn is_unc(target: &str) -> bool {
    target.starts_with("\\\\")
}

/// Shared prefix of both routers: commands and UNC paths behave identically.
fn plan_common(target: &str, raw_param: &str) -> Option<LaunchPlan> {
    if target.trim().is_empty() {
        return Some(LaunchPlan::Nothing);
    }

    if let Some(kind) = detect_command_type(target) {
        let mut command = extract_command(target);
        if !raw_param.is_empty() {
            command = crate::expand::expand(&command, raw_param);
        }
        return Some(LaunchPlan::Command { kind, command });
    }

    if is_unc(target) {
        return Some(LaunchPlan::Explorer {
            args: format!("\"{target}\""),
        });
    }

    if has_uri_scheme(target) {
        return Some(LaunchPlan::Shell {
            file: target.to_string(),
            params: None,
        });
    }

    None
}

/// `MainViewModel.ExecuteRegularShortcut` — the path taken when the user picks
/// a shortcut from the launcher.
pub fn plan_regular(target: &str, raw_param: &str) -> LaunchPlan {
    if let Some(plan) = plan_common(target, raw_param) {
        return plan;
    }

    if Path::new(target).is_file() {
        return LaunchPlan::Shell {
            file: target.to_string(),
            params: None,
        };
    }

    // Absolute paths that exist as directories are still valid `file:` URIs in
    // .NET, so they open in Explorer rather than going through `/select`.
    if Path::new(target).is_dir() {
        return LaunchPlan::Explorer {
            args: format!("\"{target}\""),
        };
    }

    LaunchPlan::Explorer {
        args: format!("/select,\"{target}\""),
    }
}

/// `ChainedShortcutExecutor.LaunchTarget` — the path taken for each item of a chain.
pub fn plan_chain_item(target: &str, raw_param: &str) -> LaunchPlan {
    if let Some(plan) = plan_common(target, raw_param) {
        return plan;
    }

    if Path::new(target).is_file() {
        // Only chains forward the raw parameter as process arguments, and only
        // for executables.
        let is_exe = Path::new(target)
            .extension()
            .map(|e| e.eq_ignore_ascii_case("exe"))
            .unwrap_or(false);
        return LaunchPlan::Shell {
            file: target.to_string(),
            params: if is_exe && !raw_param.is_empty() {
                Some(raw_param.to_string())
            } else {
                None
            },
        };
    }

    if Path::new(target).is_dir() {
        return LaunchPlan::Explorer {
            args: format!("\"{target}\""),
        };
    }

    LaunchPlan::Shell {
        file: target.to_string(),
        params: None,
    }
}

/// Error surfaced when a launch fails.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("{0}")]
    Failed(String),
}

/// Runs a [`LaunchPlan`].
///
/// `working_directory` is the user profile in the C# (`SpecialFolder.UserProfile`);
/// pass `None` to let Windows pick.
#[cfg(windows)]
pub fn execute_plan(plan: &LaunchPlan, working_directory: Option<&str>) -> Result<(), LaunchError> {
    match plan {
        LaunchPlan::Nothing => Ok(()),
        LaunchPlan::Shell { file, params } => {
            shell_execute(file, params.as_deref(), working_directory)
        }
        LaunchPlan::Explorer { args } => shell_execute("explorer.exe", Some(args), None),
        LaunchPlan::Command { kind, command } => shell_execute(
            kind.executable(),
            Some(&kind.arguments(command)),
            working_directory,
        ),
    }
}

#[cfg(not(windows))]
pub fn execute_plan(
    _plan: &LaunchPlan,
    _working_directory: Option<&str>,
) -> Result<(), LaunchError> {
    Err(LaunchError::Failed(
        "launching is only implemented for Windows".into(),
    ))
}

/// `ShellExecuteW(NULL, "open", file, params, dir, SW_SHOWNORMAL)`.
///
/// This is exactly what `UseShellExecute = true` does under the hood, including
/// passing `params` through to the target unparsed.
#[cfg(windows)]
pub fn shell_execute(
    file: &str,
    params: Option<&str>,
    working_directory: Option<&str>,
) -> Result<(), LaunchError> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let verb = HSTRING::from("open");
    let file_w = HSTRING::from(file);
    let params_w = params.map(HSTRING::from);
    let dir_w = working_directory.map(HSTRING::from);

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file_w.as_ptr()),
            params_w
                .as_ref()
                .map(|p| PCWSTR(p.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            dir_w
                .as_ref()
                .map(|d| PCWSTR(d.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns a fake HINSTANCE; anything <= 32 is an error code.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(LaunchError::Failed(format!(
            "ShellExecuteW failed for '{file}' (code {})",
            result.0 as usize
        )))
    }
}

/// `Environment.GetFolderPath(SpecialFolder.UserProfile)` — the working
/// directory the C# hands to every shell command.
pub fn user_profile_dir() -> Option<String> {
    std::env::var("USERPROFILE").ok().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_shell_executed_directly() {
        assert_eq!(
            plan_regular("https://qgocommand.com/", ""),
            LaunchPlan::Shell {
                file: "https://qgocommand.com/".into(),
                params: None
            }
        );
    }

    #[test]
    fn custom_protocols_are_recognised_but_drive_letters_are_not() {
        assert!(has_uri_scheme("steam://run/440"));
        assert!(has_uri_scheme("ms-settings:display"));
        assert!(has_uri_scheme("mailto:a@b.com"));
        // A single-letter prefix is a drive, so it must not be treated as a scheme.
        assert!(!has_uri_scheme("C:\\Windows\\notepad.exe"));
        assert!(!has_uri_scheme("notepad.exe"));
        assert!(!has_uri_scheme("\\\\server\\share"));
    }

    #[test]
    fn unc_paths_go_straight_to_explorer_without_touching_the_filesystem() {
        assert_eq!(
            plan_regular("\\\\nas\\media\\My Files", ""),
            LaunchPlan::Explorer {
                args: "\"\\\\nas\\media\\My Files\"".into()
            }
        );
    }

    #[test]
    fn commands_are_routed_before_anything_else() {
        assert_eq!(
            plan_regular("ps: Get-Process", ""),
            LaunchPlan::Command {
                kind: CommandType::PowerShell,
                command: "Get-Process".into()
            }
        );
    }

    #[test]
    fn a_parameter_is_substituted_into_the_extracted_command() {
        assert_eq!(
            plan_regular("ps: Get-Process {name}", "code"),
            LaunchPlan::Command {
                kind: CommandType::PowerShell,
                command: "Get-Process code".into()
            }
        );
    }

    #[test]
    fn unknown_targets_differ_between_the_two_call_sites() {
        // The launcher reveals the missing path in Explorer...
        assert_eq!(
            plan_regular("Z:\\nope\\missing", ""),
            LaunchPlan::Explorer {
                args: "/select,\"Z:\\nope\\missing\"".into()
            }
        );
        // ...while a chain item hands it to the shell and lets that fail.
        assert_eq!(
            plan_chain_item("Z:\\nope\\missing", ""),
            LaunchPlan::Shell {
                file: "Z:\\nope\\missing".into(),
                params: None
            }
        );
    }

    #[test]
    fn empty_targets_do_nothing() {
        assert_eq!(plan_regular("   ", ""), LaunchPlan::Nothing);
        assert_eq!(plan_chain_item("", "x"), LaunchPlan::Nothing);
    }

    #[test]
    fn chain_items_forward_raw_parameters_to_executables() {
        // Uses this crate's own binary path so the file genuinely exists.
        let exe = std::env::current_exe().unwrap();
        let exe_str = exe.to_string_lossy().to_string();
        match plan_chain_item(&exe_str, "--flag") {
            LaunchPlan::Shell { params, .. } => {
                let expected = exe
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false);
                assert_eq!(params.is_some(), expected);
            }
            other => panic!("expected a Shell plan, got {other:?}"),
        }
        // The launcher never forwards parameters this way.
        assert_eq!(
            plan_regular(&exe_str, "--flag"),
            LaunchPlan::Shell {
                file: exe_str,
                params: None
            }
        );
    }
}
