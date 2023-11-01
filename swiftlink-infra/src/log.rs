use std::{env, io, path::Path};

use tracing::{
    dispatcher::{set_default, set_global_default},
    subscriber::DefaultGuard,
    Dispatch,
};
use tracing_subscriber::{
    fmt::{writer::MakeWriterExt, MakeWriter},
    prelude::__tracing_subscriber_SubscriberExt,
    EnvFilter,
};

pub use tracing::{debug, error, info, trace, warn, Level};

type MappedFile = crate::mapped_file::MutexMappedFile;

pub fn init_global_default<P: AsRef<Path>>(
    path: P,
    level: tracing::Level,
    filter: Option<&str>,
    size: u64,
    num: u64,
    mode: Option<u32>,
) -> DefaultGuard {
    let file = MappedFile::open(path.as_ref(), size, Some(num as usize), mode);

    let writable = file
        .0
        .lock()
        .unwrap()
        .touch()
        .map(|_| true)
        .unwrap_or_else(|err| {
            warn!("{:?}, {:?}", path.as_ref(), err);
            false
        });

    let console_level = console_level();
    let console_writer = io::stdout.with_max_level(console_level);

    let dispatch = if writable {
        let file_writer =
            MappedFile::open(path.as_ref(), size, Some(num as usize), mode).with_max_level(level);

        make_dispatch(
            level.max(console_level),
            filter,
            file_writer.and(console_writer),
        )
    } else {
        make_dispatch(console_level, filter, console_writer)
    };

    let guard = set_default(&dispatch);

    set_global_default(dispatch).expect("");
    guard
}

pub fn default() -> DefaultGuard {
    let console_level = console_level();
    let console_writer = io::stdout.with_max_level(console_level);
    set_default(&make_dispatch(console_level, None, console_writer))
}

#[inline]
fn make_dispatch<W: for<'writer> MakeWriter<'writer> + 'static + Send + Sync>(
    level: tracing::Level,
    filter: Option<&str>,
    writer: W,
) -> Dispatch {
    let fmt = tracing_subscriber::fmt::format()
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);
    let layer = tracing_subscriber::fmt::layer()
        .event_format(fmt)
        .with_writer(writer);

    Dispatch::from(
        tracing_subscriber::registry()
            .with(layer)
            .with(make_filter(level, filter)),
    )
}

fn console_level() -> Level {
    if std::env::args().any(|arg| arg == "--debug") {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    }
}

#[inline]
fn make_filter(level: tracing::Level, filter: Option<&str>) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(tracing::Level::WARN.into())
        .parse(all_swiftlink(level, filter))
        .expect("failed to configure tracing/logging")
}

#[inline]
fn all_swiftlink(level: impl ToString, filter: Option<&str>) -> String {
    filter
        .unwrap_or("swiftlink-cli={level},swiftlink={level},{env}")
        .replace("{level}", level.to_string().to_uppercase().as_str())
        .replace("{env}", get_env().as_str())
}

#[inline]
fn get_env() -> String {
    env::var("RUST_LOG").unwrap_or_default()
}

impl<'a> MakeWriter<'a> for MappedFile {
    type Writer = &'a MappedFile;
    fn make_writer(&'a self) -> Self::Writer {
        self
    }
}
