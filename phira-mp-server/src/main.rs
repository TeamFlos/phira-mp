mod l10n;

mod room;
pub use room::*;

mod server;
pub use server::*;

mod session;
pub use session::*;

use anyhow::Result;
use std::{
    collections::{
        hash_map::{Entry, VacantEntry},
        HashMap,
    },
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    path::Path,
};
use tokio::{net::TcpListener, sync::RwLock};
use tracing::{info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use uuid::Uuid;
use std::env;

pub type SafeMap<K, V> = RwLock<HashMap<K, V>>;
pub type IdMap<V> = SafeMap<Uuid, V>;

fn vacant_entry<V>(map: &mut HashMap<Uuid, V>) -> VacantEntry<'_, Uuid, V> {
    let mut id = Uuid::new_v4();
    while map.contains_key(&id) {
        id = Uuid::new_v4();
    }
    match map.entry(id) {
        Entry::Vacant(entry) => entry,
        _ => unreachable!(),
    }
}

pub fn init_log(file: &str) -> Result<WorkerGuard> {
    use tracing::{metadata::LevelFilter, Level};
    use tracing_log::LogTracer;
    use tracing_subscriber::{filter, fmt, prelude::*, EnvFilter};

    let log_dir = Path::new("log");
    if log_dir.exists() {
        if !log_dir.is_dir() {
            panic!("log exists and is not a folder");
        }
    } else {
        std::fs::create_dir(log_dir).expect("failed to create log folder");
    }

    LogTracer::init()?;

    let (non_blocking, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::hourly(log_dir, file));

    let subscriber = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_filter(LevelFilter::DEBUG),
        )
        .with(
            fmt::layer()
                .with_writer(std::io::stdout)
                .with_filter(EnvFilter::from_default_env()),
        )
        .with(
            filter::Targets::new()
                .with_target("hyper", Level::INFO)
                .with_target("rustls", Level::INFO)
                .with_target("isahc", Level::INFO)
                .with_default(Level::TRACE),
        );

    tracing::subscriber::set_global_default(subscriber).expect("unable to set global subscriber");
    Ok(guard)
}

fn get_port() -> u16 {
    match env::var_os("PHIRA_PORT") {
        Some(v) => v.into_string().unwrap().parse::<u16>().unwrap(),
        None => 23333
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _guard = init_log("phira-mp")?;

    let port = get_port();
    let addrs: &[SocketAddr] = &[
        SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port),
        SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port),
    ];

    let listener: Server = TcpListener::bind(addrs).await?.into();
    info!("Server running on port {}", port);
    loop {
        if let Err(err) = listener.accept().await {
            warn!("failed to accept: {err:?}");
        }
    }
}
