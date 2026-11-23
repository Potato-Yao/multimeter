use std::error::Error;

use multimeter::api::{ApiOptions, ApiServer};
use multimeter::browser;
use multimeter::config::FRONTEND_BASE_URL;
use multimeter::elevation;
use multimeter::provider::MultimeterInfoSource;

fn main() -> Result<(), Box<dyn Error>> {
    if elevation::relaunch_if_needed()? {
        println!("launched multimeter with administrator permissions; exiting");
        return Ok(());
    }

    println!("multimeter version {}", env!("CARGO_PKG_VERSION"));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let source = MultimeterInfoSource::probe().into_shared();
    let (server, bootstrap) = ApiServer::bind(source, ApiOptions::default()).await?;
    let launch_url = bootstrap.configured_launch_url()?;

    println!("multimeter API listening at {}", bootstrap.websocket_url());
    println!("configured frontend base URL: {FRONTEND_BASE_URL}");
    println!("launch URL: {launch_url}");
    browser::launch(&launch_url)?;

    tokio::select! {
        result = server.run() => result?,
        signal = tokio::signal::ctrl_c() => signal?,
    }
    Ok(())
}
