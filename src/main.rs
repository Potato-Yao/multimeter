use std::error::Error;
use std::sync::Arc;

use multimeter::api::{ApiOptions, ApiServer, UnavailableInfoSource};
use multimeter::config::FRONTEND_BASE_URL;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (server, bootstrap) =
        ApiServer::bind(Arc::new(UnavailableInfoSource), ApiOptions::default()).await?;

    println!("multimeter API listening at {}", bootstrap.websocket_url());
    println!("configured frontend base URL: {FRONTEND_BASE_URL}");
    println!(
        "development launch URL: {}",
        bootstrap.configured_launch_url()?
    );

    tokio::select! {
        result = server.run() => result?,
        signal = tokio::signal::ctrl_c() => signal?,
    }
    Ok(())
}
