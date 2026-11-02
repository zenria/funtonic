//! File upload handling: writes uploaded files on the executor host and reports what changed.

use grpc_service::grpc_protocol::task_output::Output;
use grpc_service::grpc_protocol::{UploadFiles, UploadedFile};
use similar::TextDiff;
use std::fs::{self, Metadata, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use thiserror::Error;

/// Mode applied to new files when the commander did not provide one.
const DEFAULT_MODE: u32 = 0o644;

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("destination must be an absolute path: {0}")]
    RelativeDestination(String),
    #[error("destination {0} must be an existing directory when uploading several files")]
    DestinationNotADirectory(String),
    #[error("destination directory {0} does not exist")]
    MissingDestinationDirectory(String),
    #[error("invalid file name {0:?}")]
    InvalidFileName(String),
    #[error("{0} has no parent directory")]
    NoParent(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Outcome of an upload task: lines to report to the commander, and the global status.
#[derive(Debug)]
pub struct UploadReport {
    pub lines: Vec<Output>,
    pub success: bool,
}

/// Write every uploaded file, never stopping on the first error: each failure is reported
/// on stderr and makes the whole upload unsuccessful.
pub fn upload_files(upload: &UploadFiles) -> UploadReport {
    let dest_is_dir = match check_destination(upload) {
        Ok(dest_is_dir) => dest_is_dir,
        Err(e) => {
            return UploadReport {
                lines: vec![Output::Stderr(format!("error: {e}"))],
                success: false,
            }
        }
    };
    let destination = Path::new(&upload.destination);

    upload.files.iter().fold(
        UploadReport {
            lines: Vec::new(),
            success: true,
        },
        |mut report, file| {
            let target = if dest_is_dir {
                destination.join(&file.name)
            } else {
                destination.to_path_buf()
            };
            match upload_file(&target, file, upload.no_diff) {
                Ok(lines) => report.lines.extend(lines.into_iter().map(Output::Stdout)),
                Err(e) => {
                    report.success = false;
                    report
                        .lines
                        .push(Output::Stderr(format!("error: {}: {e}", target.display())));
                }
            }
            report
        },
    )
}

/// Validate the destination, returns whether it is an existing directory.
fn check_destination(upload: &UploadFiles) -> Result<bool, UploadError> {
    let destination = Path::new(&upload.destination);
    if !destination.is_absolute() {
        return Err(UploadError::RelativeDestination(upload.destination.clone()));
    }
    let dest_is_dir = destination.is_dir();
    if !dest_is_dir {
        if upload.files.len() > 1 {
            return Err(UploadError::DestinationNotADirectory(
                upload.destination.clone(),
            ));
        }
        if upload.destination.ends_with('/') {
            return Err(UploadError::MissingDestinationDirectory(
                upload.destination.clone(),
            ));
        }
    }
    Ok(dest_is_dir)
}

/// Write a single file and return the lines describing the change.
fn upload_file(
    target: &Path,
    file: &UploadedFile,
    no_diff: bool,
) -> Result<Vec<String>, UploadError> {
    if file.name.is_empty() || file.name == "." || file.name == ".." || file.name.contains('/') {
        return Err(UploadError::InvalidFileName(file.name.clone()));
    }
    // write through symlinks instead of replacing them
    let target = match fs::canonicalize(target) {
        Ok(resolved) => resolved,
        Err(e) if e.kind() == io::ErrorKind::NotFound => target.to_path_buf(),
        Err(e) => return Err(e.into()),
    };
    let display = target.display().to_string();

    let existing = match fs::read(&target) {
        Ok(content) => Some((content, fs::metadata(&target)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };

    let lines = match &existing {
        None => vec![format!(
            "new file: {display} ({} bytes)",
            file.content.len()
        )],
        Some((old, _)) if *old == file.content => return Ok(vec![format!("unchanged: {display}")]),
        Some(_) if no_diff => vec![format!("updated: {display}")],
        Some((old, _)) => match (as_text(old), as_text(&file.content)) {
            (Some(old), Some(new)) => TextDiff::from_lines(old, new)
                .unified_diff()
                .context_radius(3)
                .header(&display, &display)
                .to_string()
                .lines()
                .map(String::from)
                .collect(),
            _ => vec![format!(
                "updated: {display} (binary, {} -> {} bytes)",
                old.len(),
                file.content.len()
            )],
        },
    };

    write_atomically(
        &target,
        &file.content,
        existing.as_ref().map(|(_, meta)| meta),
        file.mode,
    )?;
    Ok(lines)
}

/// Content is considered as text when it is valid UTF-8 without any NUL byte.
fn as_text(content: &[u8]) -> Option<&str> {
    std::str::from_utf8(content)
        .ok()
        .filter(|text| !text.contains('\0'))
}

/// Write `content` to a temporary file next to `target` then rename it, so readers never see
/// a partially written file. The mode & owner of a replaced file are preserved.
fn write_atomically(
    target: &PathBuf,
    content: &[u8],
    existing: Option<&Metadata>,
    mode: u32,
) -> Result<(), UploadError> {
    let parent = target
        .parent()
        .ok_or_else(|| UploadError::NoParent(target.display().to_string()))?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    tmp.write_all(content)?;
    tmp.as_file().sync_all()?;

    let mode = match (existing, mode) {
        (Some(meta), _) => meta.mode(),
        (None, 0) => DEFAULT_MODE,
        (None, mode) => mode,
    };
    fs::set_permissions(tmp.path(), Permissions::from_mode(mode & 0o7777))?;

    if let Some(meta) = existing {
        let tmp_meta = tmp.as_file().metadata()?;
        if tmp_meta.uid() != meta.uid() || tmp_meta.gid() != meta.gid() {
            if let Err(e) = std::os::unix::fs::chown(tmp.path(), Some(meta.uid()), Some(meta.gid()))
            {
                warn!("Unable to preserve owner of {}: {e}", target.display());
            }
        }
    }

    tmp.persist(target).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn file(name: &str, content: &[u8]) -> UploadedFile {
        UploadedFile {
            name: name.into(),
            content: content.to_vec(),
            mode: 0o600,
        }
    }

    fn upload(destination: &Path, files: Vec<UploadedFile>, no_diff: bool) -> UploadReport {
        upload_files(&UploadFiles {
            destination: destination.to_string_lossy().into_owned(),
            files,
            no_diff,
        })
    }

    fn stdout(report: &UploadReport) -> Vec<&str> {
        report
            .lines
            .iter()
            .filter_map(|line| match line {
                Output::Stdout(line) => Some(line.as_str()),
                Output::Stderr(_) => None,
            })
            .collect()
    }

    #[test]
    fn new_file_in_directory() {
        let dir = tempdir().unwrap();
        let report = upload(dir.path(), vec![file("a.txt", b"hello\n")], false);
        assert!(report.success, "{report:?}");
        let target = dir.path().join("a.txt");
        assert_eq!(fs::read(&target).unwrap(), b"hello\n");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o600);
        let lines = stdout(&report);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("new file:"), "{lines:?}");
    }

    #[test]
    fn single_file_to_file_path() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("renamed.txt");
        let report = upload(&target, vec![file("a.txt", b"hello\n")], false);
        assert!(report.success, "{report:?}");
        assert_eq!(fs::read(&target).unwrap(), b"hello\n");
    }

    #[test]
    fn unchanged_text_diff_and_no_diff() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("a.txt");
        fs::write(&target, "line1\nline2\n").unwrap();
        fs::set_permissions(&target, Permissions::from_mode(0o640)).unwrap();

        let report = upload(dir.path(), vec![file("a.txt", b"line1\nline2\n")], false);
        assert!(report.success);
        assert!(stdout(&report)[0].starts_with("unchanged:"));

        let report = upload(dir.path(), vec![file("a.txt", b"line1\nCHANGED\n")], false);
        assert!(report.success);
        let lines = stdout(&report);
        assert!(lines.iter().any(|l| l.starts_with("@@")), "{lines:?}");
        assert!(lines.contains(&"-line2"), "{lines:?}");
        assert!(lines.contains(&"+CHANGED"), "{lines:?}");
        assert_eq!(fs::read(&target).unwrap(), b"line1\nCHANGED\n");
        // mode of the replaced file is preserved
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o640);

        let report = upload(dir.path(), vec![file("a.txt", b"other\n")], true);
        assert!(report.success);
        let lines = stdout(&report);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("updated:"), "{lines:?}");
        assert_eq!(fs::read(&target).unwrap(), b"other\n");
    }

    #[test]
    fn binary_file_is_not_diffed() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("b.bin"), b"\0\x01\x02").unwrap();
        let report = upload(dir.path(), vec![file("b.bin", b"\0\x03")], false);
        assert!(report.success);
        let lines = stdout(&report);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("binary"), "{lines:?}");
    }

    #[test]
    fn several_files_require_a_directory() {
        let dir = tempdir().unwrap();
        let report = upload(
            &dir.path().join("not_a_dir"),
            vec![file("a", b"a"), file("b", b"b")],
            false,
        );
        assert!(!report.success);
        assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn relative_destination_and_missing_directory_are_rejected() {
        let report = upload(Path::new("relative"), vec![file("a", b"a")], false);
        assert!(!report.success);

        let dir = tempdir().unwrap();
        let missing = format!("{}/missing/", dir.path().display());
        let report = upload(Path::new(&missing), vec![file("a", b"a")], false);
        assert!(!report.success);
    }

    #[test]
    fn errors_do_not_stop_other_files() {
        let dir = tempdir().unwrap();
        let report = upload(
            dir.path(),
            vec![file("..", b"evil"), file("ok.txt", b"ok")],
            false,
        );
        assert!(!report.success);
        assert_eq!(fs::read(dir.path().join("ok.txt")).unwrap(), b"ok");
        assert!(report
            .lines
            .iter()
            .any(|l| matches!(l, Output::Stderr(e) if e.contains("invalid file name"))));
    }
}
