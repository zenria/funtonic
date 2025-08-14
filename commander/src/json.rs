use std::collections::HashMap;

use serde_json::{json, Value};

use crate::cmd::JsonMode;

pub struct JsonCollector {
    json_mode: JsonMode,
    inner: HashMap<String, JsonCollectorInner>,
}

impl JsonCollector {
    pub fn new(json_mode: JsonMode) -> Self {
        Self {
            json_mode,
            inner: Default::default(),
        }
    }

    fn collector(&mut self, executor: &str) -> &mut JsonCollectorInner {
        self.inner
            .entry(executor.to_string())
            .or_insert(JsonCollectorInner::new(self.json_mode))
    }

    pub fn collect_stdout(&mut self, executor: &str, data: String) {
        self.collector(executor).stdout(data);
    }
    pub fn collect_stderr(&mut self, executor: &str, data: String) {
        self.collector(executor).stderr(data, executor);
    }
    pub fn into_json(self) -> Value {
        Value::Object(
            self.inner
                .into_iter()
                .map(|(key, value)| {
                    let value = value.into_json(&key);
                    (key, value)
                })
                .collect(),
        )
    }
}

enum JsonCollectorInner {
    EscapeSeparate {
        stdout: Vec<String>,
        stderr: Vec<String>,
    },
    EscapeMerge {
        merged: Vec<String>,
    },
    StdoutJson {
        stdout: Vec<String>,
    },
}

impl JsonCollectorInner {
    fn new(mode: JsonMode) -> Self {
        match mode {
            JsonMode::EscapeSeparate => JsonCollectorInner::EscapeSeparate {
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
            JsonMode::EscapeMerge => JsonCollectorInner::EscapeMerge { merged: Vec::new() },
            JsonMode::StdoutJson => Self::StdoutJson { stdout: Vec::new() },
        }
    }

    fn stdout(&mut self, data: String) {
        match self {
            JsonCollectorInner::EscapeSeparate { stdout, stderr: _ } => stdout.push(data),
            JsonCollectorInner::EscapeMerge { merged } => merged.push(data),
            JsonCollectorInner::StdoutJson { stdout } => stdout.push(data),
        }
    }

    fn stderr(&mut self, data: String, executor: &str) {
        match self {
            JsonCollectorInner::EscapeSeparate { stdout: _, stderr } => stderr.push(data),
            JsonCollectorInner::EscapeMerge { merged } => merged.push(data),
            JsonCollectorInner::StdoutJson { stdout: _ } => eprintln!("{executor}: {data}"),
        }
    }

    fn into_json(self, executor: &str) -> Value {
        match self {
            JsonCollectorInner::EscapeSeparate { stdout, stderr } => json!({
                "stdout": stdout.join(""),
                "stderr": stderr.join("")
            }),
            JsonCollectorInner::EscapeMerge { merged } => Value::String(merged.join("")),
            JsonCollectorInner::StdoutJson { stdout } => {
                let json = stdout.join("");
                match serde_json::from_str(&json) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("{executor} - Invalid json: {e}: {json}");
                        Value::Null
                    }
                }
            }
        }
    }
}
