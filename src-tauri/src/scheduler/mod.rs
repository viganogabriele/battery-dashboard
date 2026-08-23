//! Opt-in management of the per-user systemd recorder timer.
//!
//! This module deliberately does not install binaries or unit files. A caller
//! must stage those application-owned files in the user's XDG locations before
//! calling [`SystemdUserScheduler::enable`]. The only state changes performed
//! here follow an explicit enable or disable request and use `systemctl --user`.

use std::{fmt, process::Command};

/// Name of the one-shot recorder service managed by the timer.
pub const RECORDER_SERVICE_UNIT: &str = "battery-dashboard-recorder.service";
/// Name of the opt-in timer which triggers the recorder service.
pub const RECORDER_TIMER_UNIT: &str = "battery-dashboard-recorder.timer";
/// Placeholder in the service template that an explicit installer must replace.
pub const RECORDER_PATH_PLACEHOLDER: &str = "{{RECORDER_PATH}}";
/// Packaged one-shot service template, kept separate from a user's unit file.
pub const RECORDER_SERVICE_TEMPLATE: &str =
    include_str!("../../../systemd/battery-dashboard-recorder.service");
/// Packaged 60-second timer template, kept separate from a user's unit file.
pub const RECORDER_TIMER_TEMPLATE: &str =
    include_str!("../../../systemd/battery-dashboard-recorder.timer");

/// Error returned when rendering a systemd unit template for explicit staging.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemplateError {
    /// A recorder executable path was empty, relative, or contained a line break.
    InvalidRecorderPath,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("recorder path must be an absolute path without line breaks")
    }
}

impl std::error::Error for TemplateError {}

/// Renders the service unit for a stable, user-owned recorder executable path.
///
/// Rendering is pure: it does not create directories or write a unit file. The
/// caller owns the explicit staging step into the user's XDG config directory.
///
/// # Errors
///
/// Returns [`TemplateError::InvalidRecorderPath`] when the path is not safe for
/// use in a systemd `ExecStart` directive.
pub fn render_recorder_service(recorder_path: &str) -> Result<String, TemplateError> {
    if !is_safe_absolute_path(recorder_path) {
        return Err(TemplateError::InvalidRecorderPath);
    }

    let escaped_path = recorder_path.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(
        RECORDER_SERVICE_TEMPLATE
            .replace(RECORDER_PATH_PLACEHOLDER, &format!("\"{escaped_path}\"")),
    )
}

/// The observable state of the systemd user scheduler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerStatus {
    /// The timer is installed and enabled for the current user.
    Enabled,
    /// The timer is available but not enabled for the current user.
    Disabled,
    /// `systemctl --user` could not determine the timer state.
    Unavailable {
        /// Safe diagnostic describing why the user manager cannot be reached.
        reason: String,
    },
}

/// Captured result of a process invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    /// Exit status code when the process exited normally.
    pub status: Option<i32>,
    /// Standard output, decoded lossily as UTF-8.
    pub stdout: String,
    /// Standard error, decoded lossily as UTF-8.
    pub stderr: String,
}

/// Error raised when a command could not be started.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandError {
    message: String,
}

impl CommandError {
    /// Creates a command-launch error with a safe, user-displayable message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

/// Abstraction for invoking `systemctl`, allowing deterministic tests.
pub trait CommandRunner {
    /// Runs `program` with `args` and captures its result.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the command cannot be started.
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError>;
}

/// Production command runner backed by [`std::process::Command`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessCommandRunner;

impl CommandRunner for ProcessCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|error| CommandError::new(format!("could not start {program}: {error}")))?;

        Ok(CommandOutput {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Error returned by an explicit scheduler state change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    /// No reachable systemd user manager is available.
    Unavailable(String),
    /// A required `systemctl --user` command did not succeed.
    CommandFailed {
        /// Label of the failed `systemctl --user` command.
        command: String,
        /// Safe standard-output or standard-error detail from the failure.
        detail: String,
    },
    /// The command completed but the requested state could not be confirmed.
    VerificationFailed(SchedulerStatus),
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(reason) => {
                write!(formatter, "systemd user scheduler is unavailable: {reason}")
            }
            Self::CommandFailed { command, detail } => {
                write!(formatter, "{command} failed: {detail}")
            }
            Self::VerificationFailed(status) => {
                write!(
                    formatter,
                    "could not confirm requested scheduler state: {status:?}"
                )
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

/// Systemd user scheduler facade for the Battery Dashboard recorder timer.
///
/// It never invokes `sudo` or `pkexec`, never writes unit files itself, and
/// only changes state from [`Self::enable`] or [`Self::disable`].
#[derive(Clone, Debug)]
pub struct SystemdUserScheduler<R> {
    runner: R,
}

impl<R> SystemdUserScheduler<R> {
    /// Creates a scheduler using an injectable command runner.
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
}

impl SystemdUserScheduler<ProcessCommandRunner> {
    /// Creates a scheduler that uses the current user's `systemctl` executable.
    #[must_use]
    pub fn for_current_user() -> Self {
        Self::new(ProcessCommandRunner)
    }
}

impl<R: CommandRunner> SystemdUserScheduler<R> {
    /// Reports whether the recorder timer is enabled without changing state.
    #[must_use]
    pub fn status(&self) -> SchedulerStatus {
        match self.run(&["--user", "is-enabled", "--quiet", RECORDER_TIMER_UNIT]) {
            Ok(output) if output.status == Some(0) => SchedulerStatus::Enabled,
            Ok(output) if is_disabled_unit_result(&output) => SchedulerStatus::Disabled,
            Ok(output) => SchedulerStatus::Unavailable {
                reason: command_detail(&output),
            },
            Err(error) => SchedulerStatus::Unavailable {
                reason: error.to_string(),
            },
        }
    }

    /// Reloads per-user unit definitions, enables the timer, and starts it.
    ///
    /// The caller must have already staged the recorder and unit files under
    /// user-owned XDG paths. No privileged process is started.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError`] when no systemd user manager is available,
    /// a `systemctl` invocation fails, or the enabled state cannot be verified.
    pub fn enable(&self) -> Result<(), SchedulerError> {
        self.require_available()?;
        self.run_checked(&["--user", "daemon-reload"])?;
        self.run_checked(&["--user", "enable", "--now", RECORDER_TIMER_UNIT])?;
        self.expect_status(&SchedulerStatus::Enabled)
    }

    /// Stops and disables the timer while preserving recorder data and files.
    ///
    /// This is an explicit user action. It does not delete the `SQLite` database.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError`] when no systemd user manager is available,
    /// a `systemctl` invocation fails, or the disabled state cannot be verified.
    pub fn disable(&self) -> Result<(), SchedulerError> {
        self.require_available()?;
        self.run_checked(&["--user", "disable", "--now", RECORDER_TIMER_UNIT])?;
        self.expect_status(&SchedulerStatus::Disabled)
    }

    fn require_available(&self) -> Result<(), SchedulerError> {
        match self.status() {
            SchedulerStatus::Unavailable { reason } => Err(SchedulerError::Unavailable(reason)),
            SchedulerStatus::Enabled | SchedulerStatus::Disabled => Ok(()),
        }
    }

    fn expect_status(&self, expected: &SchedulerStatus) -> Result<(), SchedulerError> {
        let actual = self.status();
        if &actual == expected {
            Ok(())
        } else {
            Err(SchedulerError::VerificationFailed(actual))
        }
    }

    fn run_checked(&self, args: &[&str]) -> Result<(), SchedulerError> {
        let output = self
            .run(args)
            .map_err(|error| SchedulerError::CommandFailed {
                command: command_label(args),
                detail: error.to_string(),
            })?;

        if output.status == Some(0) {
            Ok(())
        } else {
            Err(SchedulerError::CommandFailed {
                command: command_label(args),
                detail: command_detail(&output),
            })
        }
    }

    fn run(&self, args: &[&str]) -> Result<CommandOutput, CommandError> {
        self.runner.run("systemctl", args)
    }
}

fn is_disabled_unit_result(output: &CommandOutput) -> bool {
    if output.status != Some(1) || !output.stderr.trim().is_empty() {
        return false;
    }

    let state = output.stdout.trim();
    state.is_empty()
        || matches!(
            state,
            "disabled" | "static" | "indirect" | "generated" | "transient"
        )
}

fn command_detail(output: &CommandOutput) -> String {
    let detail = output.stderr.trim();
    if !detail.is_empty() {
        return detail.to_owned();
    }

    let detail = output.stdout.trim();
    if !detail.is_empty() {
        return detail.to_owned();
    }

    match output.status {
        Some(status) => format!("exit status {status}"),
        None => "terminated without an exit status".to_owned(),
    }
}

fn command_label(args: &[&str]) -> String {
    format!("systemctl {}", args.join(" "))
}

fn is_safe_absolute_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains(['\n', '\r', '\0'])
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use super::{
        CommandError, CommandOutput, CommandRunner, RECORDER_PATH_PLACEHOLDER,
        RECORDER_SERVICE_TEMPLATE, RECORDER_TIMER_TEMPLATE, RECORDER_TIMER_UNIT, SchedulerError,
        SchedulerStatus, SystemdUserScheduler, TemplateError, render_recorder_service,
    };

    #[derive(Default)]
    struct FakeRunner {
        responses: RefCell<VecDeque<Result<CommandOutput, CommandError>>>,
        calls: RefCell<Vec<(String, Vec<String>)>>,
    }

    impl FakeRunner {
        fn with_responses(
            responses: impl IntoIterator<Item = Result<CommandOutput, CommandError>>,
        ) -> Self {
            Self {
                responses: RefCell::new(responses.into_iter().collect()),
                calls: RefCell::default(),
            }
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.borrow().clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.iter().map(|argument| (*argument).to_owned()).collect(),
            ));
            self.responses
                .borrow_mut()
                .pop_front()
                .expect("test must provide one response per command")
        }
    }

    fn output(status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            status: Some(status),
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn status_reports_enabled_from_systemctl() {
        let scheduler =
            SystemdUserScheduler::new(FakeRunner::with_responses([Ok(output(0, "", ""))]));

        assert_eq!(scheduler.status(), SchedulerStatus::Enabled);
    }

    #[test]
    fn status_reports_disabled_when_timer_is_not_enabled() {
        let scheduler = SystemdUserScheduler::new(FakeRunner::with_responses([Ok(output(
            1,
            "disabled\n",
            "",
        ))]));

        assert_eq!(scheduler.status(), SchedulerStatus::Disabled);
    }

    #[test]
    fn status_reports_missing_user_manager_as_unavailable() {
        let scheduler = SystemdUserScheduler::new(FakeRunner::with_responses([Ok(output(
            1,
            "",
            "Failed to connect to bus: No medium found",
        ))]));

        assert_eq!(
            scheduler.status(),
            SchedulerStatus::Unavailable {
                reason: "Failed to connect to bus: No medium found".to_owned(),
            }
        );
    }

    #[test]
    fn enable_reloads_then_enables_and_verifies_timer() {
        let runner = FakeRunner::with_responses([
            Ok(output(1, "disabled\n", "")),
            Ok(output(0, "", "")),
            Ok(output(0, "", "")),
            Ok(output(0, "", "")),
        ]);
        let scheduler = SystemdUserScheduler::new(runner);

        assert_eq!(scheduler.enable(), Ok(()));
        assert_eq!(
            scheduler.runner.calls(),
            vec![
                (
                    "systemctl".to_owned(),
                    vec![
                        "--user".to_owned(),
                        "is-enabled".to_owned(),
                        "--quiet".to_owned(),
                        RECORDER_TIMER_UNIT.to_owned(),
                    ],
                ),
                (
                    "systemctl".to_owned(),
                    vec!["--user".to_owned(), "daemon-reload".to_owned()],
                ),
                (
                    "systemctl".to_owned(),
                    vec![
                        "--user".to_owned(),
                        "enable".to_owned(),
                        "--now".to_owned(),
                        RECORDER_TIMER_UNIT.to_owned(),
                    ],
                ),
                (
                    "systemctl".to_owned(),
                    vec![
                        "--user".to_owned(),
                        "is-enabled".to_owned(),
                        "--quiet".to_owned(),
                        RECORDER_TIMER_UNIT.to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn disable_stops_disables_and_verifies_timer() {
        let runner = FakeRunner::with_responses([
            Ok(output(0, "", "")),
            Ok(output(0, "", "")),
            Ok(output(1, "disabled\n", "")),
        ]);
        let scheduler = SystemdUserScheduler::new(runner);

        assert_eq!(scheduler.disable(), Ok(()));
        assert_eq!(
            scheduler.runner.calls(),
            vec![
                (
                    "systemctl".to_owned(),
                    vec![
                        "--user".to_owned(),
                        "is-enabled".to_owned(),
                        "--quiet".to_owned(),
                        RECORDER_TIMER_UNIT.to_owned(),
                    ],
                ),
                (
                    "systemctl".to_owned(),
                    vec![
                        "--user".to_owned(),
                        "disable".to_owned(),
                        "--now".to_owned(),
                        RECORDER_TIMER_UNIT.to_owned(),
                    ],
                ),
                (
                    "systemctl".to_owned(),
                    vec![
                        "--user".to_owned(),
                        "is-enabled".to_owned(),
                        "--quiet".to_owned(),
                        RECORDER_TIMER_UNIT.to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn enable_does_not_mutate_state_when_systemd_is_unavailable() {
        let runner = FakeRunner::with_responses([Err(CommandError::new("systemctl not found"))]);
        let scheduler = SystemdUserScheduler::new(runner);

        assert_eq!(
            scheduler.enable(),
            Err(SchedulerError::Unavailable(
                "systemctl not found".to_owned()
            ))
        );
        assert_eq!(scheduler.runner.calls().len(), 1);
    }

    #[test]
    fn service_template_is_rendered_for_a_stable_absolute_recorder_path() {
        let rendered =
            render_recorder_service("/home/example/.local/libexec/battery-dashboard/recorder")
                .expect("absolute paths are valid");

        assert!(!rendered.contains(RECORDER_PATH_PLACEHOLDER));
        assert!(
            rendered
                .contains("ExecStart=\"/home/example/.local/libexec/battery-dashboard/recorder\"")
        );
        assert!(rendered.contains("Type=oneshot"));
        assert!(RECORDER_SERVICE_TEMPLATE.contains(RECORDER_PATH_PLACEHOLDER));
    }

    #[test]
    fn service_template_rejects_unsafe_recorder_paths() {
        assert_eq!(
            render_recorder_service("recorder"),
            Err(TemplateError::InvalidRecorderPath)
        );
        assert_eq!(
            render_recorder_service("/home/example/recorder\nExecStart=/unexpected"),
            Err(TemplateError::InvalidRecorderPath)
        );
    }

    #[test]
    fn timer_template_is_opt_in_and_runs_the_one_shot_service_every_minute() {
        assert!(RECORDER_TIMER_TEMPLATE.contains("OnUnitActiveSec=60s"));
        assert!(RECORDER_TIMER_TEMPLATE.contains("Persistent=false"));
        assert!(RECORDER_TIMER_TEMPLATE.contains("Unit=battery-dashboard-recorder.service"));
        assert!(RECORDER_TIMER_TEMPLATE.contains("WantedBy=timers.target"));
        assert!(!RECORDER_SERVICE_TEMPLATE.contains("[Install]"));
    }
}
