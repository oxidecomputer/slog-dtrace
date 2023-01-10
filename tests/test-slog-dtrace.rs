//! Integration test for the `slog-dtrace` crate.

// Copyright 2021 Oxide Computer Company
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![cfg_attr(usdt_need_asm, feature(asm))]
#![cfg_attr(all(target_os = "macos", usdt_need_asm_sym), feature(asm_sym))]

fn main() {}

#[cfg(test)]
mod tests {
    use slog::{info, o, warn, Drain, Logger};
    use slog_dtrace::{Message, ProbeRegistration};
    use std::ffi::{OsStr, OsString};
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Duration;
    use subprocess::{Exec, Popen};

    const POST_DTRACE_WAIT: Duration = Duration::from_secs(2);
    const SUBPROC_WAIT: Duration = Duration::from_secs(5);

    fn root_command() -> OsString {
        if cfg!(target_os = "illumos") {
            "pfexec".parse().unwrap()
        } else {
            "sudo".parse().unwrap()
        }
    }

    // Required because we need to run as superuser
    fn kill_dtrace(pid: u32) {
        Command::new(root_command())
            .arg("kill")
            .arg(format!("{}", pid))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .output()
            .expect("Could not run `kill`");
    }

    fn find_dtrace() -> Option<String> {
        let result = Command::new("which")
            .arg("dtrace")
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to spawn `which`")
            .wait_with_output()
            .expect("Could not run `which`");
        if result.status.success() {
            Some(String::from_utf8(result.stdout).unwrap().trim().to_string())
        } else {
            None
        }
    }

    // Run DTrace with the given args, return the child.
    fn run_dtrace<S>(args: &[S]) -> Result<Popen, String>
    where
        S: AsRef<OsStr> + std::fmt::Debug,
    {
        let dtrace = find_dtrace().ok_or_else(|| String::from("Could not find `dtrace`"))?;
        let dtrace = Exec::cmd(root_command())
            .arg(dtrace)
            .args(args)
            .stdin(subprocess::NullFile)
            .stderr(subprocess::NullFile)
            .stdout(subprocess::Redirection::Pipe)
            .popen()
            .map_err(|e| e.to_string())?;
        std::thread::sleep(POST_DTRACE_WAIT);
        Ok(dtrace)
    }

    // Parse a message from a line, or None.
    fn read_message_from_line<S>(line: S) -> Option<Message>
    where
        S: AsRef<str>,
    {
        // The message is actually in an result-like enum, strip that bit.
        let prefix = r#"{"ok":"#;
        let suffix = "}";
        let msg = line
            .as_ref()
            .trim()
            .strip_prefix(prefix)?
            .strip_suffix(suffix)?;
        serde_json::from_str(msg).ok()
    }

    // Helper to run DTrace and emit a single warning message from a logger.
    fn run_dtrace_single_warn_message(cmd: &str) -> Option<Message> {
        let mut dtrace = run_dtrace(&["-Z", "-n", cmd, "-q"]).unwrap();

        {
            let (drain, registration) = slog_dtrace::Dtrace::new();
            assert!(registration.is_success(), "Failed to register probes");
            let log = Logger::root(drain.fuse(), o!("key" => "value"));
            warn!(log, "a message"; "some-key" => 2);
        }

        let mut communicator = dtrace.communicate_start(None).limit_time(SUBPROC_WAIT);
        match communicator.read_string() {
            Err(e) => {
                kill_dtrace(dtrace.pid().unwrap());
                panic!("{}", e);
            }
            Ok((Some(stdout), _)) => {
                dtrace
                    .wait_timeout(SUBPROC_WAIT)
                    .expect("failed to wait for dtrace child process");
                read_message_from_line(&stdout)
            }
            Ok((None, _)) => unreachable!("stdout should have been redirected"),
        }
    }

    // NOTE: These tests need to be run serially in a single thread, to avoid the `dtrace(1)` call
    // from the other test receiving the messages from this one.
    #[test]
    fn test_dtrace_alone() {
        let cmd = r#"
        slog*:::* {
            printf("%s\n", copyinstr(arg0));
            exit(0);
        }"#;
        let msg = run_dtrace_single_warn_message(cmd).expect("failed to parse a warning message");
        assert_eq!(msg.message, "a message");
        assert_eq!(msg.kv["key"], serde_json::Value::from("value"));
        assert_eq!(msg.kv["some-key"], serde_json::Value::from(2));
    }

    #[test]
    fn test_dtrace_specific_level() {
        let cmd = r#"
        slog*:::warn {
            printf("%s\n", copyinstr(arg0));
            exit(0);
        }"#;
        let msg = run_dtrace_single_warn_message(cmd).expect("failed to parse a warning message");
        assert_eq!(msg.message, "a message");
        assert_eq!(msg.kv["key"], serde_json::Value::from("value"));
        assert_eq!(msg.kv["some-key"], serde_json::Value::from(2));
    }

    #[test]
    fn test_dtrace_wrong_level() {
        // The warn probe is needed so that DTrace will exit when the warning message is emitted.
        let cmd = r#"
        slog*:::trace { }
        slog*:::warn { exit(0); }
        "#;
        assert!(run_dtrace_single_warn_message(cmd).is_none());
    }

    #[test]
    fn test_dtrace_with_drain() {
        let mut dtrace = run_dtrace(&[
            "-Z",
            "-n",
            r#"
            BEGIN {
                self->x = 0;
            }
            slog*:::* { 
                self->x = self->x + 1;
                printf("%s\n", copyinstr(arg0));
                if (self->x == 2) {
                    exit(0);
                }
            }"#,
            "-q",
        ])
        .unwrap();

        let (writer, mut reader) = std::os::unix::net::UnixStream::pair().unwrap();
        {
            let decorator = slog_term::PlainSyncDecorator::new(writer);
            let drain = slog_term::FullFormat::new(decorator)
                .build()
                .filter_level(slog::Level::Warning)
                .fuse();
            let (drain, registration) = slog_dtrace::with_drain(drain);
            if let ProbeRegistration::Failed(ref e) = registration {
                panic!("Failed to register probes: {:#?}", e);
            }
            let log = Logger::root(drain.fuse(), o!("key" => "value"));
            warn!(log, "a message"; "some-key" => 2);
            info!(log, "just for dtrace"; "dtrace" => true);
        }

        let mut communicator = dtrace.communicate_start(None).limit_time(SUBPROC_WAIT);
        let stdout = communicator
            .read_string()
            .expect("failed to read dtrace output")
            .0
            .expect("failed to read a line from dtrace stdout");
        dtrace
            .wait_timeout(SUBPROC_WAIT)
            .expect("failed to wait for dtrace child process");

        let lines = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect::<Vec<_>>();
        let messages: Vec<Message> = lines
            .iter()
            .map(|line| read_message_from_line(&line).expect("failed to parse a message"))
            .collect();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message, "a message");
        assert_eq!(messages[0].kv["key"], serde_json::Value::from("value"));
        assert_eq!(messages[0].kv["some-key"], serde_json::Value::from(2));
        assert_eq!(messages[1].message, "just for dtrace");
        assert_eq!(messages[1].kv["dtrace"], serde_json::Value::from(true));

        // Verify that the "stdout" only received a single line, the warning message
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        let lines: Vec<_> = String::from_utf8(buf)
            .unwrap()
            .lines()
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect();
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(line.contains("WARN"));
        assert!(line.contains("some-key: 2"));
        assert!(!line.contains("dtrace"));
    }
}
