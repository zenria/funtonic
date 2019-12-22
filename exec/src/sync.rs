use crate::{ExecEvent, Line, Type};
use std::fmt::{Debug, Formatter};
use std::io;
use std::io::{Error, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

fn capture_lines<R: Read + Send + 'static>(
    reader: R,
    events_sender: crossbeam::Sender<ExecEvent>,
    line_type: Type,
) {
    std::thread::spawn(move || {
        let mut line_buffer = Vec::new();
        for byte in reader.bytes() {
            match byte {
                Ok(byte) => {
                    line_buffer.push(byte);
                    if byte == '\n' as u8 {
                        // new line, sent it to the line channel
                        let mut line = Vec::with_capacity(line_buffer.len());
                        line.append(&mut line_buffer);
                        if let Err(_) =
                            events_sender.send(ExecEvent::LineEmitted(Line { line, line_type }))
                        {
                            // channel dropped somehow
                            return;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        // if there are some remaining bytes, try to send them
        if line_buffer.len() > 0 {
            let _ = events_sender.send(ExecEvent::LineEmitted(Line {
                line: line_buffer,
                line_type,
            }));
        }
    });
}

pub fn extexec(
    mut command: Command,
) -> Result<
    (
        crossbeam::channel::Receiver<ExecEvent>,
        crossbeam::channel::Sender<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let (events_sender, event_receiver) = crossbeam::channel::unbounded();

    let (kill_sender, kill_receiver) = crossbeam::channel::bounded(1);

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    events_sender.send(ExecEvent::Started).unwrap();

    capture_lines(
        child.stdout.take().unwrap(),
        events_sender.clone(),
        Type::Out,
    );
    capture_lines(
        child.stderr.take().unwrap(),
        events_sender.clone(),
        Type::Err,
    );
    std::thread::spawn(move || loop {
        match child.try_wait() {
            Ok(exit_status) => {
                if let Some(exit_status) = exit_status {
                    let _ = events_sender.send(ExecEvent::Finished(exit_status.code().unwrap()));
                    break;
                }
            }
            Err(_) => break,
        }
        if let Ok(_kill) = kill_receiver.try_recv() {
            warn!("killed task!");
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    });
    Ok((event_receiver, kill_sender))
}

pub fn exec_command(
    command: &str,
) -> Result<
    (
        crossbeam::channel::Receiver<ExecEvent>,
        crossbeam::channel::Sender<()>,
    ),
    Box<dyn std::error::Error>,
> {
    if cfg!(target_os = "windows") {
        let mut cmd = std::process::Command::new("cmd");

        cmd.args(&["/C", &command]);
        extexec(cmd)
    } else {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(&command);
        extexec(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::Type::Out;
    use super::*;
    use std::process::Command;
    trait ExecEventHelper {
        fn line(s: &str, line_type: Type) -> ExecEvent;
        fn out(s: &str) -> ExecEvent;
        fn err(s: &str) -> ExecEvent;
    }
    impl ExecEventHelper for ExecEvent {
        fn line(s: &str, line_type: Type) -> ExecEvent {
            ExecEvent::LineEmitted(Line {
                line_type,
                line: s.as_bytes().to_vec(),
            })
        }
        fn out(s: &str) -> ExecEvent {
            Self::line(s, Type::Out)
        }
        fn err(s: &str) -> ExecEvent {
            Self::line(s, Type::Err)
        }
    }

    #[test]
    fn stdout() {
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg("echo coucou");

        let output: Vec<ExecEvent> = extexec(cmd).unwrap().0.iter().collect();
        assert_eq!(
            vec![
                ExecEvent::Started,
                ExecEvent::out("coucou\n"),
                ExecEvent::Finished(0)
            ],
            output
        );
    }
    #[test]
    fn stderr() {
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(">&2 echo coucou");
        let output: Vec<ExecEvent> = extexec(cmd).unwrap().0.iter().collect();
        assert_eq!(
            vec![
                ExecEvent::Started,
                ExecEvent::err("coucou\n"),
                ExecEvent::Finished(0)
            ],
            output
        );
    }

    #[test]
    fn stderrnout() {
        let mut cmd = Command::new("bash");
        cmd.arg("-c")
            .arg("echo foo\nsleep 1\n>&2 echo coucou\nsleep 1\necho bar");
        let output: Vec<ExecEvent> = extexec(cmd).unwrap().0.iter().collect();
        assert_eq!(
            vec![
                ExecEvent::Started,
                ExecEvent::out("foo\n"),
                ExecEvent::err("coucou\n"),
                ExecEvent::out("bar\n"),
                ExecEvent::Finished(0)
            ],
            output
        );
        // same without tee output
        let mut cmd = Command::new("bash");
        cmd.arg("-c")
            .arg("echo foo\nsleep 1\n>&2 echo coucou\nsleep 1\necho bar");
        let output: Vec<ExecEvent> = extexec(cmd).unwrap().0.iter().collect();
        assert_eq!(
            vec![
                ExecEvent::Started,
                ExecEvent::out("foo\n"),
                ExecEvent::err("coucou\n"),
                ExecEvent::out("bar\n"),
                ExecEvent::Finished(0)
            ],
            output
        );
    }
}
