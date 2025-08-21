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

struct EmailInfo {
    from: String,
    subject: String,
}

struct App {
    emails: Vec<EmailInfo>,
}

pub async fn run(token: google_api::ApiToken) -> Result<()> {
    // top 50 message ID's
    let message_list = google_api::list_messages(&token).await?;
    let message_ids = message_list.messages.unwrap_or_default();

    // fetch details {sender, title} for each message
    let detail_futures = message_ids
        .iter()
        .map(|msg| google_api::get_message_details(&token, &msg.id));

    let details_results = join_all(detail_futures).await;

    let emails: Vec<EmailInfo> = details_results
        .into_iter()
        .filter_map(Result::ok) 
        .map(|detail| EmailInfo {
            from: detail.get_header("From"),
            subject: detail.get_header("Subject"),
        })
        .collect();

    let app = App { emails };

    // start tui
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|f| {
            let header_cells = ["From", "Subject"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().bold().underlined()));
            let header = Row::new(header_cells).height(1);

            let rows = app.emails.iter().map(|email| {
                Row::new(vec![
                    Cell::from(email.from.clone()),
                    Cell::from(email.subject.clone()),
                ])
            });

            let table = Table::new(
                rows,
                [
                    Constraint::Percentage(30),
                    Constraint::Percentage(70),
                ],
            )
            .header(header)
            .block(Block::default().borders(Borders::ALL).title("Inbox (Top 50)"));

            f.render_widget(table, f.area());
        })?;

        // exit with q
        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}