use clap::{Parser, Subcommand};
mod tui;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    // list of emails from inbox
    List,
}

fn main() {
    let args = Args::parse();

    match args.command {
        Commands::List => {
            if let Err(e) = tui::run() {
                eprintln!("Error: {:?}", e);
            }
        }
    }
}
