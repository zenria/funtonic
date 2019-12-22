#[macro_use]
extern crate log;

use std::fmt::{Debug, Formatter};
use std::process::ExitStatus;

pub mod sync;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Type {
    Out,
    Err,
}

#[derive(Eq, PartialEq)]
pub struct Line {
    pub line_type: Type,
    pub line: Vec<u8>,
}
#[derive(Eq, PartialEq, Debug)]
pub enum ExecEvent {
    Started,
    Finished(i32),
    LineEmitted(Line),
}

impl Debug for Line {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "{:?}({})",
            self.line_type,
            String::from_utf8_lossy(&self.line)
        )
    }
}

pub struct Output {
    pub exit_status: ExitStatus,
    pub output_lines: Vec<Line>,
}
