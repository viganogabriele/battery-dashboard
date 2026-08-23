//! Explicit staging of the recorder executable and user-owned systemd units.
//!
//! Staging happens only after a user enables background recording. It writes
//! below XDG data/config locations and never invokes a privileged helper.

use std::{
    env,
    error::Error,
    fmt, fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use crate::{scheduler, storage};

const APPLICATION_DIRECTORY: &str = "battery-dashboard";
const RECORDER_FILE_NAME: &str = "battery-dashboard-recorder";

/// Paths staged when a user explicitly enables background recording.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecorderInstallation {
    /// Stable executable path referenced by the rendered systemd service.
    pub recorder_path: PathBuf,
    /// Per-user one-shot service unit path.
    pub service_path: PathBuf,
    /// Per-user 60-second timer unit path.
    pub timer_path: PathBuf,
}

/// An error while locating or explicitly staging recorder support files.
#[derive(Debug)]
pub enum RecorderInstallError {
    /// Neither XDG nor home-directory variables identified a user config home.
    ConfigDirectoryUnavailable,
    /// The built recorder executable is absent beside the desktop executable.
    RecorderBinaryUnavailable(PathBuf),
    /// A path required by systemd could not be represented as UTF-8.
    NonUnicodePath(PathBuf),
    /// The service template rejected a destination path.
    Template(scheduler::TemplateError),
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// Resolving the user data directory failed.
    Storage(storage::StorageError),
}

impl fmt::Display for RecorderInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigDirectoryUnavailable => formatter.write_str(
                "could not resolve XDG_CONFIG_HOME; set XDG_CONFIG_HOME or HOME before enabling recording",
            ),
            Self::RecorderBinaryUnavailable(path) => write!(
                formatter,
                "the recorder executable is not available at {}; build or install it before enabling recording",
                path.display()
            ),
            Self::NonUnicodePath(path) => {
                write!(formatter, "the recorder path is not valid UTF-8: {}", path.display())
            }
            Self::Template(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
        }
    }
}

impl Error for RecorderInstallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Template(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::ConfigDirectoryUnavailable
            | Self::RecorderBinaryUnavailable(_)
            | Self::NonUnicodePath(_) => None,
        }
    }
}

impl From<std::io::Error> for RecorderInstallError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<scheduler::TemplateError> for RecorderInstallError {
    fn from(error: scheduler::TemplateError) -> Self {
        Self::Template(error)
    }
}

impl From<storage::StorageError> for RecorderInstallError {
    fn from(error: storage::StorageError) -> Self {
        Self::Storage(error)
    }
}

/// Stages the recorder that was built alongside the running desktop executable.
///
/// Release packaging must place `battery-dashboard-recorder` beside the desktop
/// executable. During development it can be created with
/// `cargo build --bin battery-dashboard-recorder` before enabling recording.
///
/// # Errors
///
/// Returns [`RecorderInstallError`] when the recorder binary is absent, an XDG
/// path cannot be resolved, or the explicit staging operation cannot complete.
pub fn stage_built_recorder() -> Result<RecorderInstallation, RecorderInstallError> {
    let current = env::current_exe()?;
    let parent = current
        .parent()
        .ok_or_else(|| RecorderInstallError::RecorderBinaryUnavailable(current.clone()))?;
    stage_recorder_from(parent.join(RECORDER_FILE_NAME))
}

/// Stages a specific built recorder executable into stable user-owned paths.
///
/// This explicit injection point is used by tests and by installers. It does
/// not enable or start the timer; callers must invoke the scheduler separately.
///
/// # Errors
///
/// Returns [`RecorderInstallError`] when the source is absent, an XDG path
/// cannot be resolved, or a staged file cannot be written atomically.
pub fn stage_recorder_from(
    recorder_source: impl AsRef<Path>,
) -> Result<RecorderInstallation, RecorderInstallError> {
    let data_home = data_home()?;
    let config_home = config_home()?;
    stage_recorder_at(recorder_source, data_home, config_home)
}

fn stage_recorder_at(
    recorder_source: impl AsRef<Path>,
    data_home: impl AsRef<Path>,
    config_home: impl AsRef<Path>,
) -> Result<RecorderInstallation, RecorderInstallError> {
    let recorder_source = recorder_source.as_ref();
    if !recorder_source.is_file() {
        return Err(RecorderInstallError::RecorderBinaryUnavailable(
            recorder_source.to_owned(),
        ));
    }

    let recorder_path = data_home
        .as_ref()
        .join(APPLICATION_DIRECTORY)
        .join("libexec")
        .join(RECORDER_FILE_NAME);
    let unit_directory = config_home.as_ref().join("systemd/user");
    let service_path = unit_directory.join(scheduler::RECORDER_SERVICE_UNIT);
    let timer_path = unit_directory.join(scheduler::RECORDER_TIMER_UNIT);
    let recorder_path_string = recorder_path
        .to_str()
        .ok_or_else(|| RecorderInstallError::NonUnicodePath(recorder_path.clone()))?;
    let rendered_service = scheduler::render_recorder_service(recorder_path_string)?;
    let recorder_bytes = fs::read(recorder_source)?;

    atomic_write(&recorder_path, &recorder_bytes, 0o700)?;
    atomic_write(&service_path, rendered_service.as_bytes(), 0o600)?;
    atomic_write(
        &timer_path,
        scheduler::RECORDER_TIMER_TEMPLATE.as_bytes(),
        0o600,
    )?;

    Ok(RecorderInstallation {
        recorder_path,
        service_path,
        timer_path,
    })
}

fn data_home() -> Result<PathBuf, RecorderInstallError> {
    let database_path = storage::default_database_path()?;
    database_path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_owned)
        .ok_or_else(|| {
            RecorderInstallError::Storage(storage::StorageError::DataDirectoryUnavailable)
        })
}

fn config_home() -> Result<PathBuf, RecorderInstallError> {
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(config_home));
    }

    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".config"))
        .ok_or(RecorderInstallError::ConfigDirectoryUnavailable)
}

fn atomic_write(path: &Path, contents: &[u8], mode: u32) -> Result<(), RecorderInstallError> {
    let parent = path.parent().ok_or_else(|| {
        RecorderInstallError::Io(std::io::Error::other("staged path has no parent directory"))
    })?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("recorder"),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    set_permissions(&temporary, mode)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<(), RecorderInstallError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _mode: u32) -> Result<(), RecorderInstallError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::stage_recorder_at;

    fn temporary_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is valid")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "battery-dashboard-installer-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn staging_writes_a_private_recorder_and_rendered_user_units() {
        let root = temporary_path("stage");
        let source = root.join("build/battery-dashboard-recorder");
        fs::create_dir_all(source.parent().expect("source has a parent"))
            .expect("source directory is created");
        fs::write(&source, b"recorder bytes").expect("source binary is created");

        let installation = stage_recorder_at(&source, root.join("data"), root.join("config"))
            .expect("recorder support stages");
        assert_eq!(
            fs::read(&installation.recorder_path).expect("recorder is readable"),
            b"recorder bytes"
        );
        let service = fs::read_to_string(&installation.service_path).expect("service is readable");
        assert!(
            service.contains(
                installation
                    .recorder_path
                    .to_str()
                    .expect("UTF-8 test path")
            )
        );
        assert!(installation.timer_path.is_file());

        fs::remove_dir_all(root).expect("test directory is removed");
    }
}
