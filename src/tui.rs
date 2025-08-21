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
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Terminal,
};
use std::io::stdout;

enum AppMode {
    List,
    Viewing,
}

struct EmailInfo {
    id: String,
    from: String,
    subject: String,
}

// UPDATED: The App struct now tracks the scroll offset
struct App {
    mode: AppMode,
    emails: Vec<EmailInfo>,
    selected_index: usize,
    current_email_body: String,
    scroll_offset: u16,
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

    // UPDATED: Helper functions for scrolling
    fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }
}

pub async fn run(token: google_api::ApiToken) -> Result<()> {
    // --- Data Fetching ---
    let message_list = google_api::list_messages(&token).await?;
    let message_ids = message_list.messages.unwrap_or_default();

    let header_futures = message_ids
        .iter()
        .map(|msg| google_api::get_message_details(&token, &msg.id));
    let header_results = join_all(header_futures).await;

    let emails: Vec<EmailInfo> = header_results
        .into_iter()
        .filter_map(Result::ok)
        .map(|detail| EmailInfo {
            id: detail.id.clone(),
            from: detail.get_header("From"),
            subject: detail.get_header("Subject"),
        })
        .collect();

    let mut app = App {
        mode: AppMode::List,
        emails,
        selected_index: 0,
        current_email_body: String::new(),
        scroll_offset: 0, // UPDATED: Initialize scroll offset
    };

    // --- TUI Initialization ---
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // --- Main Loop ---
    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(f.area());

            match app.mode {
                AppMode::List => {
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

                    let table = Table::new(rows, [Constraint::Percentage(30), Constraint::Percentage(70)])
                        .header(header)
                        .block(Block::default().borders(Borders::ALL).title("Inbox (Top 50)"));
                    
                    f.render_widget(table, chunks[0]);
                }
                AppMode::Viewing => {
                    // UPDATED: The Paragraph widget is now scrollable
                    let email_view = Paragraph::new(app.current_email_body.as_str())
                        .block(Block::default().borders(Borders::ALL).title("Email Content"))
                        .wrap(Wrap { trim: false }) // Use trim: false for better scroll experience
                        .scroll((app.scroll_offset, 0)); // Tell the widget how much to scroll
                    f.render_widget(email_view, chunks[0]);
                }
            }

            // --- Footer Bar ---
            let footer_text = match app.mode {
                AppMode::List => "↑/↓: Navigate  |  Enter: View Email  |  q: Quit",
                AppMode::Viewing => "↑/↓: Scroll  |  q: Back to List",
            };
            let footer = Paragraph::new(footer_text)
                .style(Style::default().fg(Color::White).bg(Color::DarkGray));
            f.render_widget(footer, chunks[1]);
        })?;

        // --- Event Handling ---
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app.mode {
                    AppMode::List => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Down => app.next(),
                        KeyCode::Up => app.previous(),
                        KeyCode::Enter => {
                            if let Some(selected_email) = app.emails.get(app.selected_index) {
                                let detail = google_api::get_message_details(&token, &selected_email.id).await?;
                                app.current_email_body = google_api::decode_email_body(&detail);
                                app.scroll_offset = 0; // UPDATED: Reset scroll on new email
                                app.mode = AppMode::Viewing;
                            }
                        }
                        _ => {}
                    },
                    AppMode::Viewing => match key.code {
                        // UPDATED: Handle scrolling and quitting
                        KeyCode::Char('q') => {
                            app.mode = AppMode::List;
                        }
                        KeyCode::Down => app.scroll_down(),
                        KeyCode::Up => app.scroll_up(),
                        _ => {}
                    },
                }
            }
        }
    }

    // --- TUI Cleanup ---
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}