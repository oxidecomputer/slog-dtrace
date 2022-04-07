# slog-dtrace

[![Latest Version]][crates.io] [![Documentation]][docs.rs]

Forward `slog` messages as DTrace USDT probes.

## Overview

Logging is invaluable in production applications. However, it presents a bit of a quandary.
Most of the time, only informational or error messages are useful. But when an application
crashes or is misbehaving, it can be extremely useful to retrieve more verbose logging
information. Unfortunately, this can normally only be accomplished by restarting the process
with a new log level.

This crate allows applications to attach a `slog::Drain`, the `Dtrace` drain, to their
loggers that forwards all messages to DTrace. This is done with a
[`usdt`](https://docs.rs/usdt/latest) probe function, with different probes indicating
different log levels.

Note that the `Dtrace` drain will _only_ send messages to DTrace, but in most situations, one
is already sending log messages to some location (stdout, file, syslog, etc.). The
`with_drain` constructor can be used to generate a `Dtrace` drain that will forward
messages to an existing drain as well as to DTrace.

The DTrace probe that emits log messages is efficient. In particular, when the probe is
disabled, it incurs no cost beyond that of any other drain(s) in the hierarchy. However, when
the probe is enabled, every message, regardless of log-level, can be viewed in DTrace.

## Example

```bash
$ cargo +nightly run --example simple

```

You can see that only warning messages are printed in the terminal. However, running a DTrace
command in another shell should reveal more messages.

```bash
# dtrace -Z -n 'slog*::: { printf("%s\n", copyinstr(arg0)); }' -q
{"ok": {"location":{"module":"simple","file":"examples/simple.rs","line":15},"level":"WARN","timestamp":"2021-10-19T17:55:55.260393409Z","message":"a warning message for everyone","kv":{"cool":true,"key":"value"}}}
{"ok": {"location":{"module":"simple","file":"examples/simple.rs","line":16},"level":"INFO","timestamp":"2021-10-19T17:55:55.260531762Z","message":"info is just for dtrace","kv":{"cool":true,"hello":"from dtrace","key":"value"}}}
{"ok": {"location":{"module":"simple","file":"examples/simple.rs","line":17},"level":"DEBUG","timestamp":"2021-10-19T17:55:55.260579423Z","message":"only dtrace gets debug messages","kv":{"cool":true,"hello":"from dtrace","key":"value"}}}
```

We can see both the warning messages that the example's stdout prints, but also an info and
debug message. There are specific probes for each logging level, allowing users to run DTrace
actions in response to specific levels of messages. For example, this DTrace command receives
just messages emitted via the `debug!` logging macro.

```bash
# dtrace -Z -n 'slog*:::debug { printf("%s\n", copyinstr(arg0)); }' -q
{"ok": {"location":{"module":"simple","file":"examples/simple.rs","line":17},"level":"DEBUG","timestamp":"2021-10-19T17:57:30.578681933Z","message":"only dtrace gets debug messages","kv":{"cool":true,"hello":"from dtrace","key":"value"}}}
```

## Notes

This crate inherits a reliance on a nightly toolchain from the `usdt` crate.

[Latest Version]: https://img.shields.io/crates/v/slog-dtrace.svg
[crates.io]: https://crates.io/crates/slog-dtrace
[Documentation]: https://docs.rs/slog-dtrace/badge.svg
[docs.rs]: https://docs.rs/slog-dtrace
