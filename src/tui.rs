use crate::google_api;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::future::join_all;
use ratatui::{
    backend::CrosstermBackend,
    layout::Constraint,
    style::{Style, Stylize},
    widgets::{Block, Borders, Cell, Row, Table},
    Terminal,
};
use std::io::stdout;

// A clean struct to hold just what the TUI needs
struct EmailInfo {
    from: String,
    subject: String,
}

// The App struct now holds a Vec of the detailed EmailInfo
struct App {
    emails: Vec<EmailInfo>,
}

pub async fn run(token: google_api::ApiToken) -> Result<()> {
    // 1. Get the list of the top 50 message IDs
    let message_list = google_api::list_messages(&token).await?;
    let message_ids = message_list.messages.unwrap_or_default();

    // 2. Create tasks to fetch details for each message concurrently
    let detail_futures = message_ids
        .iter()
        .map(|msg| google_api::get_message_details(&token, &msg.id));

    // 3. Run all the tasks at once and wait for them to complete
    let details_results = join_all(detail_futures).await;

    // 4. Process the results into our clean EmailInfo struct
    let emails: Vec<EmailInfo> = details_results
        .into_iter()
        .filter_map(Result::ok) // Keep only the successful fetches
        .map(|detail| EmailInfo {
            from: detail.get_header("From"),
            subject: detail.get_header("Subject"),
        })
        .collect();

    // Initialize the App with the fetched email details
    let app = App { emails };

    // --- TUI Initialization ---
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // --- Main UI Loop ---
    loop {
        terminal.draw(|f| {
            // Define headers for our table
            let header_cells = ["From", "Subject"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().bold().underlined()));
            let header = Row::new(header_cells).height(1);

            // Create a row for each email
            let rows = app.emails.iter().map(|email| {
                Row::new(vec![
                    Cell::from(email.from.clone()),
                    Cell::from(email.subject.clone()),
                ])
            });

            // Create the table widget
            let table = Table::new(
                rows,
                [
                    // Define column widths
                    Constraint::Percentage(30),
                    Constraint::Percentage(70),
                ],
            )
            .header(header)
            .block(Block::default().borders(Borders::ALL).title("Inbox (Top 50)"));

            // Render the table in the full terminal area
            f.render_widget(table, f.size());
        })?;

        // Handle keyboard input (non-blocking)
        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }

    // --- TUI Cleanup ---
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}