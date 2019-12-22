use crate::{ExecEvent, Line, Type};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub enum InternalError {}

fn exec_command(command: &str) -> Result<UnboundedReceiver<ExecEvent>, Box<dyn std::error::Error>> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();

    let stdout = child
        .stdout()
        .take()
        .expect("child did not have a handle to stdout");
    let stderr = child
        .stderr()
        .take()
        .expect("child did not have a handle to stdout");

    tokio::spawn(wait_for_exit(child, sender.clone()));
    tokio::spawn(read_output_stream(Type::Out, stdout, sender.clone()));
    tokio::spawn(read_output_stream(Type::Err, stderr, sender));

    Ok(receiver)
}

async fn wait_for_exit(child: Child, sender: UnboundedSender<ExecEvent>) {
    if let Err(e) = sender.send(ExecEvent::Started) {
        // this should not happen however
        warn!("Unable to send started event {}", e)
    }
    let status = child.await.expect("child process encountered an error");
    // FIXME unwrap() here
    if let Err(e) = sender.send(ExecEvent::Finished(status.code().unwrap())) {
        // this should not happen however
        warn!("Unable to send finished execution result {}", e)
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
