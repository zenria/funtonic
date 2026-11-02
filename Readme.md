# funtonic

Testing [tonic](https://github.com/hyperium/tonic) Rust gRPC server/client

Command & control unix boxes.

## Security model

Connections between components (executor <-> taskserver, commander <-> taskserver) are secured
using mTLS.

Each command issued by the `commander` is signed using a private key. The `taskserver` verifies the command payload signature
and if it is not valid or if the key is not known, the command is rejected.

If a command also targets executors, each `executor`, also verifies the command payload signature and if it's not valid or
the key is not known, the command is rejected.

All commands are executed by the user executing by the `executor`.

Executors also authenticates themselves to the `taskserver` using a private keypair ; the `executor` will not receive any
command until its key has been accepted by the `taskserver`

NOTE: there is no end to end encryption of commands nor results sent back&forth to executors. The taskserver is fully aware
of the content of the command and its results.

## Single command execution

```

Commander           Taskserver              Executor
                         <------- GetTasks ----+
                         |
   ------ LaunchTask --->+
                         +---- GetTaskReply -->+            
                                               | task starts on executor
   <- LaunchTaskResponse +<-- TaskExecution ---| executor reports events & output
   <- LaunchTaskResponse +<-- TaskExecution ---| back to the Task server...
   <- LaunchTaskResponse +<-- TaskExecution ---|

```

## File upload

```sh
commander upload [--no-diff] <query> <files>... <dest>
```

Like `scp`, the last argument is the remote destination. It must be an absolute path. If it is an existing directory,
files are written inside it. Otherwise (a single file only), it is the path of the written file. Missing parent
directories are not created.

Files are written atomically (temporary file + rename). A replaced file keeps its mode and owner. A new file gets the
mode of the local file.

Each executor reports, for each file:
- `new file: <path>` when the file did not exist
- `unchanged: <path>` when the content is identical (the file is not rewritten)
- a unified diff when both versions are text, or `updated: <path>` when `--no-diff` is used or the file is binary

The output is always grouped by executor. The upload has to fit into a single gRPC message (64MiB).
