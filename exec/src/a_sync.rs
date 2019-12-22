use crate::{ExecEvent, Line, Type};
use futures::future::join_all;
use futures::{select, FutureExt};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot::{Receiver, Sender};
use tokio::task::JoinHandle;

#[derive(thiserror::Error, Debug)]
pub enum InternalError {
    #[error("Unable to get stdout handle")]
    NoStdOut,
    #[error("Unable to get stderr handle")]
    NoStdErr,
}

pub fn exec_command(
    command: &str,
) -> Result<(UnboundedReceiver<ExecEvent>, Sender<()>), Box<dyn std::error::Error>> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true) // needed to allow the command to be killed on kill event
        .spawn()?;

    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let (kill_sender, kill_receiver) = tokio::sync::oneshot::channel::<()>();

    let stdout = child.stdout().take().ok_or(InternalError::NoStdOut)?;
    let stderr = child.stderr().take().ok_or(InternalError::NoStdErr)?;

    let stdout_join = tokio::spawn(read_output_stream(Type::Out, stdout, sender.clone()));
    let stderr_join = tokio::spawn(read_output_stream(Type::Err, stderr, sender.clone()));
    tokio::spawn(wait_for_exit(
        child,
        kill_receiver,
        sender.clone(),
        vec![stdout_join, stderr_join],
    ));

    Ok((receiver, kill_sender))
}

async fn wait_for_exit(
    child: Child,
    kill_recv: Receiver<()>,
    sender: UnboundedSender<ExecEvent>,
    streams_join: Vec<JoinHandle<()>>,
) {
    if let Err(e) = sender.send(ExecEvent::Started) {
        // this should not happen however
        warn!("Unable to send started event {}", e)
    }
    let mut kill_recv = kill_recv.fuse();

    select! {
        _ = join_all(streams_join).fuse() => (),
        _ = kill_recv => return
    }

    let mut child = child.fuse();

    select! {
        status = child =>{
            let status = status.expect("child process encountered an error");
            if let Err(e) = sender.send(ExecEvent::Finished(status.code().unwrap())) {
                // this should not happen however
                warn!("Unable to send finished execution result {}", e)
            }
        }
        _ = kill_recv => {
            // this function will exit, thus the child future will be dropped and
            // and the child process will be killed
            return;
        }
    }
}

async fn read_output_stream<T: AsyncRead + Unpin>(
    stream_type: Type,
    stream: T,
    sender: UnboundedSender<ExecEvent>,
) {
    let mut reader = BufReader::new(stream).lines();
    loop {
        match reader.next_line().await {
            Ok(maybe_line) => {
                match maybe_line {
                    Some(line) => {
                        if let Err(e) = sender.send(ExecEvent::LineEmitted(Line {
                            line_type: stream_type,
                            line: line.as_bytes().to_vec(),
                        })) {
                            // this should not happen however
                            warn!("Unable to send finished execution result {}", e)
                        }
                    }
                    None => break, // EOF
                }
            }
            Err(e) => {
                error!("Unable to read stream {}", e);
                break;
            }
        }
    }
}
#[cfg(test)]
mod test {
    use super::*;
    use crate::*;
    use futures::stream::StreamExt;
    use log::LevelFilter;

    #[tokio::test]
    async fn test() {
        assert_eq!(
            exec_command("echo foo ; echo bar")
                .unwrap()
                .0
                .collect::<Vec<ExecEvent>>()
                .await,
            vec![
                ExecEvent::Started,
                ExecEvent::out("foo"),
                ExecEvent::out("bar"),
                ExecEvent::Finished(0)
            ],
        );

        assert_eq!(
            exec_command("echo foo ; exit 123")
                .unwrap()
                .0
                .collect::<Vec<ExecEvent>>()
                .await,
            vec![
                ExecEvent::Started,
                ExecEvent::out("foo"),
                ExecEvent::Finished(123)
            ],
        );

        assert_eq!(
            exec_command(">&2 echo bar ; exit 5")
                .unwrap()
                .0
                .collect::<Vec<ExecEvent>>()
                .await,
            vec![
                ExecEvent::Started,
                ExecEvent::err("bar"),
                ExecEvent::Finished(5)
            ],
        );
    }
}
