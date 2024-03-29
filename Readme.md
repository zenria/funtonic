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
