use tokio::runtime::{Builder, Runtime};
use tracing::{info, warn};

#[cfg(feature = "multicore")]
pub(crate) fn build() -> Runtime {
    let mut cores = std::env::var("SWIFTLINK_CORES")
        .ok()
        .and_then(|v| {
            let opt = v.parse::<usize>().ok().filter(|n| *n > 0);
            if opt.is_none() {
                warn!(SWIFTLINK_CORES = %v,"Ignoring invalid configuration");
            }
            opt
        })
        .unwrap_or(0);

    let cpus = num_cpus::get();
    debug_assert!(cpus > 0, "At least one CPU must be available");
    if cores > cpus {
        warn!(
            cpus,
            SWIFTLINK_CORES = cores,
            "Ignoring configuration due to insufficient resources"
        );
        cores = cpus;
    }

    match cores {
        // `0` is unexpected, but it's a wild world out there.
        0 | 1 => {
            info!("Using single-threaded tokio runtime");
            Builder::new_current_thread()
                .enable_all()
                .thread_name("swiftlink-runtime")
                .build()
                .expect("Failed to build basic runtime!")
        }
        num_cpus => {
            info!(%cores,"Using multi-threaded tokio runtime");
            Builder::new_multi_thread()
                .enable_all()
                .thread_name("swiftlink-runtime")
                .worker_threads(num_cpus)
                .max_blocking_threads(num_cpus)
                .build()
                .expect("Failed to build threaded runtime!")
        }
    }
}

#[cfg(not(feature = "multicore"))]
pub(crate) fn build() -> Runtime {
    Builder::new()
        .enable_all()
        .basic_scheduler()
        .thread_name("swiftlink-runtime")
        .build()
        .expect("failed to build basic runtime!")
}
