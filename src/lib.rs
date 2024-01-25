//! Forward `slog` messages as DTrace USDT probes.
//!
//! Logging is invaluable in production applications. However, it presents a bit of a quandary.
//! Most of the time, only informational or error messages are useful. But when an application
//! crashes or is misbehaving, it can be extremely useful to retrieve more verbose logging
//! information. Unfortunately, this can normally only be accomplished by restarting the process
//! with a new log level.
//!
//! This crate allows applications to attach a [`slog::Drain`], the `Dtrace` drain, to their
//! loggers that forwards all messages to DTrace. This is done with a
//! [`usdt`](https://docs.rs/usdt/latest) probe function, with different probes indicating
//! different log levels.
//!
//! Note that the [`Dtrace`] drain will _only_ send messages to DTrace, but in most situations, one
//! is already sending log messages to some location (stdout, file, syslog, etc.). The
//! [`with_drain`] constructor can be used to generate a [`Dtrace`] drain that will forward
//! messages to an existing drain as well as to DTrace.
//!
//! The DTrace probe that emits log messages is efficient. In particular, when the probe is
//! disabled, it incurs no cost beyond that of any other drain(s) in the hierarchy. However, when
//! the probe is enabled, every message, regardless of log-level, can be viewed in DTrace.
//!
//! Example
//! -------
//!
//! ```bash
//! $ cargo +nightly run --example simple
//!
//! ```
//!
//! You can see that only warning messages are printed in the terminal. However, running a DTrace
//! command in another shell should reveal more messages.
//!
//! ```bash
//! ## dtrace -Z -n 'slog*::: { printf("%s\n", copyinstr(arg0)); }' -q
//! {"ok": {"location":{"module":"simple","file":"examples/simple.rs","line":15},"level":"WARN","timestamp":"2021-10-19T17:55:55.260393409Z","message":"a warning message for everyone","kv":{"cool":true,"key":"value"}}}
//! {"ok": {"location":{"module":"simple","file":"examples/simple.rs","line":16},"level":"INFO","timestamp":"2021-10-19T17:55:55.260531762Z","message":"info is just for dtrace","kv":{"cool":true,"hello":"from dtrace","key":"value"}}}
//! {"ok": {"location":{"module":"simple","file":"examples/simple.rs","line":17},"level":"DEBUG","timestamp":"2021-10-19T17:55:55.260579423Z","message":"only dtrace gets debug messages","kv":{"cool":true,"hello":"from dtrace","key":"value"}}}
//! ```
//!
//! We can see both the warning messages that the example's stdout prints, but also an info and
//! debug message. There are specific probes for each logging level, allowing users to run DTrace
//! actions in response to specific levels of messages. For example, this DTrace command receives
//! just messages emitted via the `debug!` logging macro.
//!
//! ```bash
//! ## dtrace -Z -n 'slog*:::debug { printf("%s\n", copyinstr(arg0)); }' -q
//! {"ok": {"location":{"module":"simple","file":"examples/simple.rs","line":17},"level":"DEBUG","timestamp":"2021-10-19T17:57:30.578681933Z","message":"only dtrace gets debug messages","kv":{"cool":true,"hello":"from dtrace","key":"value"}}}
//! ```
//!
//! Notes
//! -----
//!
//! This crate inherits a reliance on a nightly toolchain from the `usdt` crate.

// Copyright 2022 Oxide Computer Company
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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use slog::{Drain, KV};

/// Type alias for a generic JSON map.
pub type JsonMap = serde_json::Map<String, serde_json::Value>;

#[usdt::provider(provider = "slog", probe_format = "{probe}_")]
mod probes {
    use crate::Message;
    fn trace(msg: &Message) {}
    fn debug(msg: &Message) {}
    fn info(msg: &Message) {}
    fn warn(msg: &Message) {}
    fn error(msg: &Message) {}
    fn critical(msg: &Message) {}
}

/// `Location` describes the location in the source from which a log message was issued.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Location {
    /// The Rust module from which the message was issued.
    pub module: String,

    /// The source file from which the message was issued.
    pub file: String,

    /// The line of the source file from which the message was issued.
    pub line: u32,
}

/// A `Message` captures the all information about a single log message.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Message {
    /// The information about the [`Location`] of a message in the source.
    pub location: Location,

    /// The logging level, see [`slog::Level`].
    pub level: String,

    /// The timestamp at which the message was issued.
    ///
    /// As there may be latencies between a message's emission and consumption in DTrace, this can
    /// be useful.
    pub timestamp: DateTime<Utc>,

    /// The string message emitted in the log entry.
    pub message: String,

    /// The key-value pairs in this log message, including those of parent loggers.
    pub kv: JsonMap,
}

/// `ProbeRegistration` stores the result of registering probes with the DTrace kernel module.
///
/// Though unlikely, it's possible that probe registration fails. This may happen, for example, if
/// the DTrace kernel module is extremely low on memory. One may want to abort the application in
/// this case, or one might decide that a running but degraded application is better than nothing i
/// such a situation. The `ProbeRegistration` enum contains information about whether probes were
/// successfully registered. The caller may decide how to handle such a case.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum ProbeRegistration {
    Success,
    Failed(String),
}

impl ProbeRegistration {
    /// Helper to check if the variant is `Success`.
    pub fn is_success(&self) -> bool {
        matches!(self, ProbeRegistration::Success)
    }
}

/// A [`slog::Drain`] that forwards all log messages to DTrace.
#[derive(Debug)]
pub struct Dtrace<D> {
    _phantom: std::marker::PhantomData<D>,
}

impl Dtrace<slog::Discard> {
    /// Create a new DTrace logger, emitting messages only to DTrace.
    ///
    /// This method will create a `Dtrace` drain that sends messages _only_ to DTrace. If you wish
    /// to emit messages to another location as well, you can use [`with_drain`] or call
    /// [`slog::Duplicate`].
    ///
    /// Note that it's possible for probe registration to fail. The result of registering is
    /// returned as the second tuple element. It may be inspected so that callers can decide how to
    /// handle failure. See [`ProbeRegistration`] for more information.
    pub fn new() -> (Self, ProbeRegistration) {
        let registration = match usdt::register_probes() {
            Ok(_) => ProbeRegistration::Success,
            Err(e) => ProbeRegistration::Failed(e.to_string()),
        };
        (
            Self {
                _phantom: std::marker::PhantomData,
            },
            registration,
        )
    }
}

/// Combine the [`Dtrace`] drain with another drain.
///
/// This duplicates all log messages to `drain` and a new `Dtrace` drain.
///
/// Note that probe registration can fail, see [`ProbeRegistration`] and [`Dtrace::new`] for more
/// information.
pub fn with_drain<D>(drain: D) -> (slog::Duplicate<D, Dtrace<slog::Discard>>, ProbeRegistration)
where
    D: Drain,
{
    let (d, registration) = Dtrace::new();
    (slog::Duplicate(drain, d), registration)
}

// Create a message to emit to DTrace
fn create_dtrace_message(record: &slog::Record, values: &slog::OwnedKVList) -> Message {
    let location = Location {
        module: record.module().to_string(),
        file: record.file().to_string(),
        line: record.line(),
    };
    let mut serializer = Serializer::default();
    let kv = match record
        .kv()
        .serialize(record, &mut serializer)
        .and_then(|_| values.serialize(record, &mut serializer))
    {
        Ok(()) => serializer.map,
        Err(e) => {
            let mut map = JsonMap::default();
            let _ = map.insert(
                String::from("err"),
                serde_json::Value::from(format!("{}", e)),
            );
            map
        }
    };
    let msg = Message {
        location,
        timestamp: Utc::now(),
        level: record.level().as_str().to_string(),
        message: record.msg().to_string(),
        kv,
    };
    msg
}

impl<D> Drain for Dtrace<D>
where
    D: Drain<Ok = (), Err = slog::Never>,
{
    type Ok = ();
    type Err = slog::Never;

    fn log(
        &self,
        record: &slog::Record<'_>,
        values: &slog::OwnedKVList,
    ) -> Result<Self::Ok, Self::Err> {
        match record.level() {
            slog::Level::Trace => probes::trace_!(|| create_dtrace_message(record, values)),
            slog::Level::Debug => probes::debug_!(|| create_dtrace_message(record, values)),
            slog::Level::Info => probes::info_!(|| create_dtrace_message(record, values)),
            slog::Level::Warning => probes::warn_!(|| create_dtrace_message(record, values)),
            slog::Level::Error => probes::error_!(|| create_dtrace_message(record, values)),
            slog::Level::Critical => probes::critical_!(|| create_dtrace_message(record, values)),
        }
        Ok(())
    }
}

// Type used to serialize slog's key-value pairs into JSON.
#[derive(Debug, Clone, Default)]
struct Serializer {
    map: crate::JsonMap,
}

impl Serializer {
    fn emit<T>(&mut self, key: slog::Key, value: T) -> slog::Result
    where
        T: Into<serde_json::Value>,
    {
        self.map.insert(key.to_string(), value.into());
        Ok(())
    }
}

macro_rules! impl_emit {
    ($method:ident, $ty:ty) => {
        fn $method(&mut self, key: slog::Key, value: $ty) -> slog::Result {
            self.emit(key, value).unwrap();
            Ok(())
        }
    };
}

impl slog::Serializer for Serializer {
    fn emit_arguments(&mut self, key: slog::Key, values: &std::fmt::Arguments<'_>) -> slog::Result {
        self.map
            .insert(key.to_string(), format!("{}", values).into());
        Ok(())
    }

    impl_emit!(emit_u8, u8);
    impl_emit!(emit_u16, u16);
    impl_emit!(emit_u32, u32);
    impl_emit!(emit_u64, u64);
    impl_emit!(emit_i8, i8);
    impl_emit!(emit_i16, i16);
    impl_emit!(emit_i32, i32);
    impl_emit!(emit_i64, i64);
    impl_emit!(emit_isize, isize);
    impl_emit!(emit_usize, usize);
    impl_emit!(emit_bool, bool);
    impl_emit!(emit_f32, f32);
    impl_emit!(emit_f64, f64);
    impl_emit!(emit_str, &str);

    fn emit_unit(&mut self, key: slog::Key) -> slog::Result {
        self.emit(key, ())
    }

    fn emit_none(&mut self, key: slog::Key) -> slog::Result {
        self.map.insert(key.to_string(), serde_json::Value::Null);
        Ok(())
    }
}
