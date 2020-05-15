use crate::path_builder::PathBuilder;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::borrow::Borrow;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub fn path_concat2<T: AsRef<Path>, U: AsRef<Path>>(p1: T, p2: U) -> PathBuf {
    PathBuilder::from_path(p1).push(p2).build()
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
#[derive(Error, Debug)]
pub enum DirCreationError {
    #[error("IO Error occured")]
    Io {
        #[from]
        source: std::io::Error,
    },
    #[error("{0} path is not a directory")]
    PathIsNotADirectory(String),
}

pub fn mkdirs<P: AsRef<Path>>(dir: P) -> Result<String, DirCreationError> {
    let dir: String = shellexpand::tilde(&dir.as_ref().to_string_lossy().into_owned()).into_owned();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        match e.kind() {
            ErrorKind::AlreadyExists => {
                // dir or file exists
                // let check the path is really a directory
                let meta = std::fs::metadata(&dir)?;
                if !meta.is_dir() {
                    Err(DirCreationError::PathIsNotADirectory(dir.clone()))?;
                }
            }
            _ => Err(e)?,
        }
    }
    Ok(dir)
}
