use clap::{Parser, Subcommand};
mod tui;
mod google_api; // Add this line

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    List,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    
    match args.command {
        Commands::List => {
            // auth token
            let auth_token = match google_api::get_auth_token().await {
                Ok(token) => token,
                Err(e) => {
                    eprintln!("Authentication error: {:?}", e);
                    return;
                }
            };
            
            
            if let Err(e) = tui::run(auth_token).await {
                eprintln!("TUI error: {:?}", e);
            }
        }
    }
}