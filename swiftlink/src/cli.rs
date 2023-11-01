use std::path::PathBuf;

use clap::{arg, command, Subcommand};

pub use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, PartialEq, Eq, Debug)]
pub enum Commands {
    /// Run the swiftlink
    Run {
        /// The path to the configuration file
        #[arg(short = 'c', long)]
        conf: Option<PathBuf>,

        /// Turn debugging information on
        #[arg(long)]
        debug: bool,
    },
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_cli_args_parse_start() {
        let cli = Cli::parse_from(["swiftlink", "run", "-c", "/etc/swiftlink.conf"]);
        assert!(matches!(
            cli.command,
            Commands::Run {
                conf: Some(_),
                debug: false
            }
        ));

        let cli = Cli::parse_from(["swiftlink", "run", "--conf", "/etc/swiftlink.conf"]);
        assert!(matches!(
            cli.command,
            Commands::Run {
                conf: Some(_),
                debug: false
            }
        ));
    }

    #[test]
    fn test_cli_args_parse_start_debug_on() {
        let cli = Cli::parse_from(["swiftlink", "run", "-c", "/etc/swiftlink.conf", "--debug"]);
        assert!(matches!(
            cli.command,
            Commands::Run {
                conf: Some(_),
                debug: true
            }
        ));
    }
}
