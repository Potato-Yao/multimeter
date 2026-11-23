use std::error::Error;
use std::sync::Arc;

use multimeter::api::{ApiOptions, ApiServer, UnavailableInfoSource};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (server, bootstrap) =
        ApiServer::bind(Arc::new(UnavailableInfoSource), ApiOptions::default()).await?;

    println!("multimeter API listening at {}", bootstrap.websocket_url());
    println!("development bootstrap fragment: {}", bootstrap.fragment()?);

    tokio::select! {
        result = server.run() => result?,
        signal = tokio::signal::ctrl_c() => signal?,
    }
    Ok(())
}
