//! Local power-profile adapter.
//!
//! The adapter talks to `powerprofilesctl` directly with a fixed executable
//! name and fixed subcommands.  It never invokes a shell and never attempts
//! privilege escalation.  A profile name is parsed into the small allowlist
//! before it can become a command argument.

use std::{io::ErrorKind, process::Command};

use serde::Serialize;

const POWERPROFILECTL: &str = "powerprofilesctl";
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Profiles understood by power-profiles-daemon.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerProfile {
    /// Prefer lower energy use.
    PowerSaver,
    /// The daemon's normal default profile.
    Balanced,
    /// Prefer performance.
    Performance,
}

impl PowerProfile {
    /// Parses exactly one supported profile name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "power-saver" => Some(Self::PowerSaver),
            "balanced" => Some(Self::Balanced),
            "performance" => Some(Self::Performance),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::PowerSaver => "power-saver",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
        }
    }
}

/// A stable response for profile discovery and changes.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerProfileResponse {
    /// Response schema version.
    pub schema_version: u8,
    /// `available`, `unsupported`, or `unavailable`.
    pub availability: &'static str,
    /// Whether the requested backend/profile operation is supported.
    pub supported: bool,
    /// Profile currently reported by the backend.
    pub active_profile: Option<PowerProfile>,
    /// Allowlisted profiles observed in the backend's profile listing.
    pub available_profiles: Vec<PowerProfile>,
    /// Profile requested by a set operation, if any.
    pub requested_profile: Option<PowerProfile>,
    /// True only after a set operation was confirmed by a fresh `get` query.
    pub changed: bool,
    /// Machine-readable explanation when the result is not available.
    pub unavailable_reason: Option<&'static str>,
    /// A concise local diagnostic, when useful.
    pub error: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
enum AdapterError {
    Unsupported(&'static str),
    Unavailable(&'static str),
}

#[derive(Debug)]
struct BackendStatus {
    active: PowerProfile,
    available: Vec<PowerProfile>,
}

/// Reads the active profile without changing system state.
#[must_use]
pub fn get_profile() -> PowerProfileResponse {
    match query_backend() {
        Ok(status) => available_response(status, None, false),
        Err(error) => error_response(error, None),
    }
}

/// Changes a profile after validating the exact allowlisted name, then
/// confirms the daemon reports that profile as active.
#[must_use]
pub fn set_profile(requested: &str) -> PowerProfileResponse {
    let Some(requested) = PowerProfile::parse(requested) else {
        return PowerProfileResponse {
            schema_version: 1,
            availability: "unavailable",
            supported: false,
            active_profile: None,
            available_profiles: Vec::new(),
            requested_profile: None,
            changed: false,
            unavailable_reason: Some("invalid-request"),
            error: Some("profile must be power-saver, balanced, or performance"),
        };
    };

    let status = match query_backend() {
        Ok(status) => status,
        Err(error) => return error_response(error, Some(requested)),
    };
    if !status.available.contains(&requested) {
        return PowerProfileResponse {
            schema_version: 1,
            availability: "unsupported",
            supported: false,
            active_profile: Some(status.active),
            available_profiles: status.available,
            requested_profile: Some(requested),
            changed: false,
            unavailable_reason: Some("profile-unsupported"),
            error: Some("the requested profile is not reported by the local backend"),
        };
    }

    let output = match invoke(&["set", requested.as_str()]) {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            // A present daemon that rejects a known profile is an unavailable
            // operation, not permission to retry with another command.
            let _ = output;
            return PowerProfileResponse {
                schema_version: 1,
                availability: "unavailable",
                supported: true,
                active_profile: Some(status.active),
                available_profiles: status.available,
                requested_profile: Some(requested),
                changed: false,
                unavailable_reason: Some("profile-change-failed"),
                error: Some("powerprofilesctl could not change the profile"),
            };
        }
        Err(AdapterError::Unsupported(reason)) => {
            return error_response(AdapterError::Unsupported(reason), Some(requested));
        }
        Err(AdapterError::Unavailable(reason)) => {
            return error_response(AdapterError::Unavailable(reason), Some(requested));
        }
    };

    // Keep the binding to the command output explicit so future changes cannot
    // accidentally treat a successful process exit as confirmation.
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return error_response(
            AdapterError::Unavailable("backend-output-too-large"),
            Some(requested),
        );
    }
    match query_backend() {
        Ok(after) if after.active == requested => available_response(after, Some(requested), true),
        Ok(after) => PowerProfileResponse {
            schema_version: 1,
            availability: "unavailable",
            supported: true,
            active_profile: Some(after.active),
            available_profiles: after.available,
            requested_profile: Some(requested),
            changed: false,
            unavailable_reason: Some("profile-change-not-confirmed"),
            error: Some("the backend did not report the requested profile as active"),
        },
        Err(error) => error_response(error, Some(requested)),
    }
}

fn query_backend() -> Result<BackendStatus, AdapterError> {
    let list = invoke(&["list"])?;
    if !list.status.success() {
        return Err(AdapterError::Unavailable("profile-list-failed"));
    }
    let mut available = parse_list_profiles(&list.stdout);

    let get = invoke(&["get"])?;
    if !get.status.success() {
        return Err(AdapterError::Unavailable("profile-query-failed"));
    }
    let Some(active) = parse_active_profile(&get.stdout) else {
        return Err(AdapterError::Unavailable("invalid-active-profile"));
    };

    // A few older versions print only the active profile for `list`.  The
    // successful `get` is still trustworthy evidence for that one profile;
    // never claim the other allowlisted profiles are available without seeing
    // them in the backend output.
    if available.is_empty() {
        available.push(active);
    } else if !available.contains(&active) {
        return Err(AdapterError::Unavailable("active-profile-not-listed"));
    }
    available.sort_by_key(|profile| profile_order(*profile));
    available.dedup();
    Ok(BackendStatus { active, available })
}

fn invoke(arguments: &[&str]) -> Result<std::process::Output, AdapterError> {
    let output = Command::new(POWERPROFILECTL)
        .args(arguments)
        .output()
        .map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                AdapterError::Unsupported("powerprofilesctl-not-found")
            } else {
                AdapterError::Unavailable("powerprofilesctl-unavailable")
            }
        })?;
    if output.stdout.len() > MAX_OUTPUT_BYTES || output.stderr.len() > MAX_OUTPUT_BYTES {
        return Err(AdapterError::Unavailable("backend-output-too-large"));
    }
    Ok(output)
}

fn parse_active_profile(output: &[u8]) -> Option<PowerProfile> {
    std::str::from_utf8(output)
        .ok()?
        .lines()
        .map(str::trim)
        .find_map(PowerProfile::parse)
}

fn parse_list_profiles(output: &[u8]) -> Vec<PowerProfile> {
    let Ok(output) = std::str::from_utf8(output) else {
        return Vec::new();
    };
    let mut profiles = Vec::new();
    for line in output.lines() {
        let line = line.trim().trim_start_matches('*').trim();
        let candidate = line
            .strip_suffix(':')
            .unwrap_or(line)
            .split_whitespace()
            .next()
            .unwrap_or_default();
        if let Some(profile) = PowerProfile::parse(candidate) {
            if !profiles.contains(&profile) {
                profiles.push(profile);
            }
        }
    }
    profiles
}

fn profile_order(profile: PowerProfile) -> u8 {
    match profile {
        PowerProfile::PowerSaver => 0,
        PowerProfile::Balanced => 1,
        PowerProfile::Performance => 2,
    }
}

fn available_response(
    status: BackendStatus,
    requested_profile: Option<PowerProfile>,
    changed: bool,
) -> PowerProfileResponse {
    PowerProfileResponse {
        schema_version: 1,
        availability: "available",
        supported: true,
        active_profile: Some(status.active),
        available_profiles: status.available,
        requested_profile,
        changed,
        unavailable_reason: None,
        error: None,
    }
}

fn error_response(
    error: AdapterError,
    requested_profile: Option<PowerProfile>,
) -> PowerProfileResponse {
    let (availability, supported, reason, message) = match error {
        AdapterError::Unsupported(reason) => (
            "unsupported",
            false,
            reason,
            "the local power-profile backend is unsupported",
        ),
        AdapterError::Unavailable(reason) => (
            "unavailable",
            true,
            reason,
            "the local power-profile backend is currently unavailable",
        ),
    };
    PowerProfileResponse {
        schema_version: 1,
        availability,
        supported,
        active_profile: None,
        available_profiles: Vec::new(),
        requested_profile,
        changed: false,
        unavailable_reason: Some(reason),
        error: Some(message),
    }
}

#[cfg(test)]
mod tests {
    use super::{PowerProfile, parse_active_profile, parse_list_profiles};

    #[test]
    fn profile_parser_is_an_exact_allowlist() {
        assert_eq!(
            PowerProfile::parse("power-saver"),
            Some(PowerProfile::PowerSaver)
        );
        assert_eq!(
            PowerProfile::parse("balanced"),
            Some(PowerProfile::Balanced)
        );
        assert_eq!(
            PowerProfile::parse("performance"),
            Some(PowerProfile::Performance)
        );
        assert_eq!(PowerProfile::parse("sudo balanced"), None);
        assert_eq!(PowerProfile::parse(""), None);
    }

    #[test]
    fn parses_standard_powerprofilesctl_listing_and_active_profile() {
        let listing = b"  power-saver:\n* balanced:\n  performance:\n    Driver: test\n";
        assert_eq!(
            parse_list_profiles(listing),
            vec![
                PowerProfile::PowerSaver,
                PowerProfile::Balanced,
                PowerProfile::Performance
            ]
        );
        assert_eq!(
            parse_active_profile(b"balanced\n"),
            Some(PowerProfile::Balanced)
        );
        assert_eq!(parse_active_profile(b"not-a-profile\n"), None);
    }
}
