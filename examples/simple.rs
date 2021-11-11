//! Example using a DTrace drain along with an existing drain.
use slog::{debug, info, o, warn, Drain, Logger};
use slog_dtrace::{with_drain, ProbeRegistration};

fn main() {
    usdt::register_probes().unwrap();
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator)
        .build()
        .filter_level(slog::Level::Warning)
        .fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let (drain, registration) = with_drain(drain);
    if let ProbeRegistration::Failed(ref e) = registration {
        panic!("Failed to register probes: {:#?}", e);
    }
    let log = Logger::root(drain.fuse(), o!("key" => "value"));
    loop {
        warn!(log, "a warning message for everyone"; "cool" => true);
        info!(log, "info is just for dtrace"; "hello" => "from dtrace", "cool" => true);
        debug!(log, "only dtrace gets debug messages"; "hello" => "from dtrace", "cool" => true);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
