use serde::de::DeserializeOwned;
use serde::Serialize;
use std::borrow::Borrow;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub fn path_concat2<T: AsRef<Path>, U: AsRef<Path>>(p1: T, p2: U) -> PathBuf {
    [p1.as_ref(), p2.as_ref().into()]
        .iter()
        .collect::<PathBuf>()
}

pub fn parse_yaml_from_file<P: AsRef<Path>, D: DeserializeOwned>(
    file: P,
) -> Result<D, anyhow::Error> {
    let file = open_file(&file)?;
    let parsed = serde_yaml::from_reader(file)?;
    Ok(parsed)
}

#[derive(Error, Debug)]
#[error("Unable to open {filename}")]
pub struct FileError {
    filename: String,
    source: std::io::Error,
}

pub fn open_file<P: AsRef<Path>>(path: P) -> Result<File, FileError> {
    match File::open(shellexpand::tilde(&path.as_ref().to_string_lossy().into_owned()).into_owned())
    {
        Ok(f) => Ok(f),
        Err(e) => Err(FileError {
            filename: path.as_ref().to_string_lossy().into(),
            source: e,
        }),
    }
}
pub fn read<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, FileError> {
    match std::fs::read(
        shellexpand::tilde(&path.as_ref().to_string_lossy().into_owned()).into_owned(),
    ) {
        Ok(f) => Ok(f),
        Err(e) => Err(FileError {
            filename: path.as_ref().to_string_lossy().into(),
            source: e,
        }),
    }
}
