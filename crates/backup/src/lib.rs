//! Keep the database directory out of OS backups.
//!
//! The database key never leaves the device, so a backed-up database could
//! not be opened after a restore: it only wastes space and keeps encrypted
//! health data around. Apple platforms (iCloud, Finder / iTunes backups,
//! Time Machine) honour `NSURLIsExcludedFromBackupKey` on the directory;
//! Android excludes through the app's backup rules and `noBackupFilesDir`;
//! other platforms have no app-level backup to opt out of.

use std::path::Path;

/// The exclusion could not be set.
#[derive(Debug)]
pub struct BackupError(pub String);

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot exclude from backups: {}", self.0)
    }
}

impl std::error::Error for BackupError {}

/// Mark `dir` (and everything in it) as excluded from backups. Call it on
/// every open: recreating the directory drops the flag.
#[cfg(any(target_os = "ios", target_os = "macos"))]
pub fn exclude_from_backup(dir: &Path) -> Result<(), BackupError> {
    use objc2_foundation::{NSNumber, NSString, NSURL, NSURLIsExcludedFromBackupKey};

    let path = dir
        .to_str()
        .ok_or_else(|| BackupError("path is not UTF-8".to_owned()))?;
    let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
    let yes = NSNumber::numberWithBool(true);
    // SAFETY: `NSURLIsExcludedFromBackupKey` is an immutable Foundation
    // constant, initialised before any Rust code runs.
    let key = unsafe { NSURLIsExcludedFromBackupKey };
    // SAFETY: this resource key expects an `NSNumber` holding a BOOL, which
    // `yes` is; `url`, `yes` and `key` stay alive for the whole call.
    unsafe { url.setResourceValue_forKey_error(Some(&yes), key) }
        .map_err(|error| BackupError(error.localizedDescription().to_string()))
}

/// Mark `dir` as excluded from backups (no-op on this platform).
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
pub const fn exclude_from_backup(_dir: &Path) -> Result<(), BackupError> {
    Ok(())
}
