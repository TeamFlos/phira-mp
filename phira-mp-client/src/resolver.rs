use anyhow::{Result, bail};
use hickory_resolver::{
    TokioResolver,
    config::{ResolverConfig, ResolverOpts},
    name_server::TokioConnectionProvider,
};
use http::uri::Authority;
use std::net::IpAddr;
use std::sync::LazyLock;

const SRV_PREFIX: &str = "_phira._tcp.";

/// Global DNS resolver reused across all lookups.
static RESOLVER: LazyLock<TokioResolver> = LazyLock::new(|| {
    TokioResolver::builder_with_config(
        ResolverConfig::default(),
        TokioConnectionProvider::default(),
    )
    .with_options(ResolverOpts::default())
    .build()
});

/// Resolves an [`Authority`] into a (host, port) pair for `TcpStream::connect`.
pub async fn resolve(auth: &Authority) -> Result<(String, u16)> {
    let host = auth.host().trim_start_matches('[').trim_end_matches(']');
    if let Some(port) = auth.port_u16() {
        Ok((host.to_string(), port))
    } else {
        resolve_srv(host).await
    }
}

async fn resolve_srv(host: &str) -> Result<(String, u16)> {
    if host.parse::<IpAddr>().is_ok() {
        bail!("Bare IP address is not supported");
    }

    let srv_name = format!("{}{}", SRV_PREFIX, host);

    let lookup = RESOLVER
        .srv_lookup(&srv_name)
        .await
        .map_err(|e| anyhow::anyhow!("SRV lookup failed: {}", e))?;

    let srv = lookup
        .iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No SRV records found"))?;

    let target = srv.target().to_string().trim_end_matches('.').to_string();
    let port = srv.port();

    Ok((target, port))
}
