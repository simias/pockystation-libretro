//! Logger implementation using libretro as a backend

use super::libretro;

use std::io::{stderr, Write};

struct RetroLogger;

impl log::Log for RetroLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let s = ::std::fmt::format(*record.args());

            let lvl = match record.level() {
                log::Level::Error => libretro::log::Level::Error,
                log::Level::Warn => libretro::log::Level::Warn,
                log::Level::Info => libretro::log::Level::Info,
                log::Level::Debug => libretro::log::Level::Debug,
                // Nothing below Debug in libretro
                log::Level::Trace => libretro::log::Level::Debug,
            };

            libretro::log::log(lvl, &s);
        }
    }

    fn flush(&self) {}
}

struct StdErrLogger;

impl log::Log for StdErrLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let _ = writeln!(&mut stderr(), "{} - {}", record.level(), record.args());
        }
    }

    fn flush(&self) {
        let _ = stderr().flush();
    }
}

pub fn init() {
    let retrolog_ok = libretro::log::init();

    let logger: Box<dyn log::Log> = if retrolog_ok {
        Box::new(RetroLogger)
    } else {
        Box::new(StdErrLogger)
    };

    log::set_boxed_logger(logger).unwrap();
    // Retroarch does its own filtering in the frontend, so it's probably not worth adding a level
    // of configuration here.
    log::set_max_level(log::LevelFilter::Debug);

    if retrolog_ok {
        info!("Logging initialized");
    } else {
        warn!("Couldn't initialize libretro logging, using stderr");
    }
}
