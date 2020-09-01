
use structopt::StructOpt;
use simplelog::{LevelFilter, SimpleLogger, TermLogger, TerminalMode};
use log::{info, error};
use tokio::stream::StreamExt;

use coap::client::{CoAPClientAsync, RequestOptions, parse_coap_url};

/// A simple utility to for interacting with CoAP services
#[derive(PartialEq, Clone, Debug, StructOpt)]
pub struct Options {
    #[structopt()]
    /// Target (hostname:port/resource) for CoAP operation
    pub target: String,

    #[structopt(flatten)]
    pub request_opts: RequestOptions,

    #[structopt(subcommand)]
    pub command: Command,

    #[structopt(long = "log-level", default_value = "info")]
    /// Configure app logging levels (warn, info, debug, trace)
    pub log_level: LevelFilter,
}

#[derive(PartialEq, Clone, Debug, StructOpt)]
pub enum Command {
    /// Perform a GET request
    Get,
    /// Perform a PUT request
    Put,
    /// Perform a POST request
    Post,
    /// Register an observer on the provided topic
    Observe,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load options
    let opts = Options::from_args();

    // Initialise logging
    let log_config = simplelog::ConfigBuilder::new().build();
    if let Err(_e) = TermLogger::init(opts.log_level, log_config.clone(), TerminalMode::Mixed) {
        SimpleLogger::init(opts.log_level, log_config).unwrap();
    }

    // TODO: handle scheme (coaps etc.)
    let (_scheme, host, port, resource) = parse_coap_url(&opts.target)?;

    // Connect CoAP client
    info!("Connecting client to target: {:?}", opts.target);
    let mut client = CoAPClientAsync::new_udp((host.as_str(), port)).await?;

    // Perform operation
    let resp = match &opts.command {
        Command::Get => client.get(&resource, &opts.request_opts).await?,
        Command::Put => client.put(&resource, &[], &opts.request_opts).await?,
        Command::Post => client.post(&resource, &[], &opts.request_opts).await?,
        Command::Observe => {
            // Setup observer
            let mut rx = client.observe(&resource, &opts.request_opts).await?;

            // Receive updates
            while let Some(resp) = rx.next().await {
                info!("Received: {:?}", resp);
            }
            
            return Ok(())
        },
    };

    info!("Response: {:?}", resp);

    Ok(())
}