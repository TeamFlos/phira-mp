mod l10n;

mod room;
pub use room::*;

mod server;
pub use server::*;

mod session;
pub use session::*;

use anyhow::Result;
use clap::Parser;
use std::{
    collections::{
        hash_map::{Entry, VacantEntry},
        HashMap,
    },
    net::{Ipv6Addr, SocketAddr},
    path::Path,
};
use tokio::{net::TcpListener, sync::RwLock};
use tracing::warn;
use tracing_appender::non_blocking::WorkerGuard;
use uuid::Uuid;

pub type SafeMap<K, V> = RwLock<HashMap<K, V>>;
pub type IdMap<V> = SafeMap<Uuid, V>;

fn vacant_entry<V>(map: &mut HashMap<Uuid, V>) -> VacantEntry<'_, Uuid, V> {
    let mut id = Uuid::new_v4();
    while map.contains_key(&id) {
        // 修正此处的语法错误
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

/// Command line arguments
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(
        short,
        long,
        default_value_t = 12346,
        help = "Specify the port number to use for the server"
    )]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _guard = init_log("phira-mp")?;

    let args = Args::parse();
    let port = args.port;
    let addrs: &[SocketAddr] = &[
        SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port),
    ];

    // 打印本地地址和端口
    for addr in addrs {
        println!("Local Address: {}", addr);
    }

    let listener: Server = TcpListener::bind(addrs).await?.into();

    loop {
        if let Err(err) = listener.accept().await {
            warn!("failed to accept: {err:?}");
        }
    }
}
