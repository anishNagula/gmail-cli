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
async fn main() { // Make `main` async
    let args = Args::parse();
    
    match args.command {
        Commands::List => {
            // Get the authentication token
            let auth_token = match google_api::get_auth_token().await {
                Ok(token) => token,
                Err(e) => {
                    eprintln!("Authentication error: {:?}", e);
                    return;
                }
            };
            
            // Now you can start the TUI and pass the auth token to it
            if let Err(e) = tui::run(auth_token).await { // Pass auth_token
                eprintln!("TUI error: {:?}", e);
            }
        }
    }
}