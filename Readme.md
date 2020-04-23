# funtonic

Testing [tonic](https://github.com/hyperium/tonic) Rust gRPC server/client 

Command & control unix boxes.

Single command execution
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
