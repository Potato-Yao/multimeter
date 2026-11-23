use std::error::Error;

use multimeter::api::{ApiOptions, ApiServer};
use multimeter::browser;
use multimeter::config::FRONTEND_BASE_URL;
use multimeter::provider::MultimeterInfoSource;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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
