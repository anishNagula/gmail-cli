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
    layout::{Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Terminal,
};
use std::io::stdout;

struct EmailInfo {
    from: String,
    subject: String,
}

struct App {
    emails: Vec<EmailInfo>,
    selected_index: usize,
}

impl App {
    fn previous(&mut self) {
        if !self.emails.is_empty() {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                self.selected_index = self.emails.len() - 1;
            }
        }
    }

    fn next(&mut self) {
        if !self.emails.is_empty() {
            if self.selected_index < self.emails.len() - 1 {
                self.selected_index += 1;
            } else {
                self.selected_index = 0;
            }
        }
    }
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

    let mut app = App {
        emails,
        selected_index: 0,
    };

    // start tui
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(0),
                    Constraint::Length(1),
                ])
                .split(f.area());

            let header_cells = ["From", "Subject"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().bold().underlined()));
            let header = Row::new(header_cells).height(1);

            let rows = app.emails.iter().enumerate().map(|(i, email)| {
                let style = if i == app.selected_index {
                    Style::default().bg(Color::Blue)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(email.from.clone()),
                    Cell::from(email.subject.clone()),
                ])
                .style(style)
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

            f.render_widget(table, chunks[0]);

            let footer_text = "↑/↓: Navigate  |  q: Quit";
            let footer = Paragraph::new(footer_text)
                .style(Style::default().fg(Color::White).bg(Color::DarkGray));
            f.render_widget(footer, chunks[1]);
        })?;

        // exit with q
        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Down => app.next(),
                    KeyCode::Up => app.previous(),
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}