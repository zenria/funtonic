#[macro_use]
extern crate log;

use std::fmt::{Debug, Formatter};
use std::process::ExitStatus;

pub mod a_sync;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Type {
    Out,
    Err,
}

#[derive(Eq, PartialEq)]
pub struct Line {
    pub line_type: Type,
    pub line: String,
}
#[derive(Eq, PartialEq, Debug)]
pub enum ExecEvent {
    Started,
    Finished(i32),
    LineEmitted(Line),
}

impl Debug for Line {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "{:?}({})", self.line_type, self.line)
    }
}

pub struct Output {
    pub exit_status: ExitStatus,
    pub output_lines: Vec<Line>,
}

#[cfg(test)]
trait ExecEventHelper {
    fn line(s: &str, line_type: Type) -> ExecEvent;
    fn out(s: &str) -> ExecEvent;
    fn err(s: &str) -> ExecEvent;
}
#[cfg(test)]
impl ExecEventHelper for ExecEvent {
    fn line(s: &str, line_type: Type) -> ExecEvent {
        ExecEvent::LineEmitted(Line {
            line_type,
            line: s.into(),
        })
    }
    fn out(s: &str) -> ExecEvent {
        Self::line(s, Type::Out)
    }
    fn err(s: &str) -> ExecEvent {
        Self::line(s, Type::Err)
    }
}
