use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Write};

use multimeter::api::{ApiOptions, ApiServer};
use multimeter::browser;
use multimeter::config::FRONTEND_BASE_URL;
use multimeter::elevation;
use multimeter::provider::MultimeterInfoSource;

const USAGE: &str = "\
Usage: multimeter [--no-browser]

Options:
  --no-browser  Start the API without opening the configured frontend URL
  -h, --help    Print this help
";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LaunchOptions {
    no_browser: bool,
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Run(LaunchOptions),
    Help,
}

fn parse_command(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut options = LaunchOptions::default();

    for argument in arguments {
        match argument.to_str() {
            Some("--no-browser") => options.no_browser = true,
            Some("-h" | "--help") => return Ok(Command::Help),
            Some(argument) => {
                return Err(format!("unknown argument `{argument}`\n\n{USAGE}"));
            }
            None => return Err(format!("argument is not valid UTF-8\n\n{USAGE}")),
        }
    }

    Ok(Command::Run(options))
}

fn print_launch_info(websocket_url: &str, launch_url: &str) -> io::Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    writeln!(stdout, "multimeter API listening at {websocket_url}")?;
    writeln!(stdout, "configured frontend base URL: {FRONTEND_BASE_URL}")?;
    writeln!(stdout, "launch URL: {launch_url}")?;
    stdout.flush()
}

fn main() -> Result<(), Box<dyn Error>> {
    let launch_options = match parse_command(env::args_os().skip(1))
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?
    {
        Command::Run(options) => options,
        Command::Help => {
            print!("{USAGE}");
            io::stdout().flush()?;
            return Ok(());
        }
    };

    if elevation::relaunch_if_needed()? {
        println!("launched multimeter with administrator permissions; exiting");
        return Ok(());
    }

    println!("multimeter version {}", env!("CARGO_PKG_VERSION"));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(launch_options))
}

async fn run(options: LaunchOptions) -> Result<(), Box<dyn Error>> {
    let source = MultimeterInfoSource::probe().into_shared();
    let (server, bootstrap) = ApiServer::bind(source, ApiOptions::default()).await?;
    let launch_url = bootstrap.configured_launch_url()?;

    print_launch_info(bootstrap.websocket_url(), &launch_url)?;
    if !options.no_browser {
        browser::launch(&launch_url)?;
    }

    tokio::select! {
        result = server.run() => result?,
        signal = tokio::signal::ctrl_c() => signal?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{Command, LaunchOptions, parse_command};

    #[test]
    fn runs_with_default_options() {
        assert_eq!(
            parse_command([]).unwrap(),
            Command::Run(LaunchOptions::default())
        );
    }

    #[test]
    fn disables_browser_launch() {
        assert_eq!(
            parse_command([OsString::from("--no-browser")]).unwrap(),
            Command::Run(LaunchOptions { no_browser: true })
        );
    }

    #[test]
    fn reports_help_without_elevating() {
        assert_eq!(
            parse_command([OsString::from("--help")]).unwrap(),
            Command::Help
        );
    }

    #[test]
    fn rejects_unknown_arguments() {
        let error = parse_command([OsString::from("--unknown")]).unwrap_err();

        assert!(error.contains("unknown argument `--unknown`"));
        assert!(error.contains("Usage: multimeter"));
    }
}
