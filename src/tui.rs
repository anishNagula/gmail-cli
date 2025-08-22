use crate::google_api;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Terminal,
};
use std::io::stdout;
use tokio::sync::mpsc;

enum AppMode {
    List,
    Viewing,
}

struct EmailInfo {
    id: String,
    from: String,
    subject: String,
    is_unread: bool,
    snippet: String,
}

struct App {
    mode: AppMode,
    is_loading: bool,
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

    fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }
}

pub async fn run(token: google_api::ApiToken) -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<EmailInfo>(100);
    let async_token = token.clone();

    tokio::spawn(async move {
        if let Ok(message_list) = google_api::list_messages(&async_token).await {
            let message_ids = message_list.messages.unwrap_or_default();
            let header_futures = message_ids
                .iter()
                .map(|msg| google_api::get_message_headers(&async_token, &msg.id));
            
            let mut results = futures::future::join_all(header_futures).await;

            for detail_result in results.drain(..) {
                if let Ok(detail) = detail_result {
                    let email_info = EmailInfo {
                        id: detail.id.clone(),
                        from: detail.get_header("From"),
                        subject: detail.get_header("Subject"),
                        is_unread: detail.is_unread(),
                        snippet: detail.snippet.clone(),
                    };
                    if tx.send(email_info).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut app = App {
        mode: AppMode::List,
        is_loading: true,
        emails: Vec::new(),
        selected_index: 0,
        current_email_body: String::new(),
        scroll_offset: 0,
    };

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        match rx.try_recv() {
            Ok(email) => app.emails.push(email),
            Err(mpsc::error::TryRecvError::Disconnected) => app.is_loading = false,
            _ => {}
        }

        terminal.draw(|f| {
            // UPDATED: The footer layout is now inside the match statement
            match app.mode {
                AppMode::List => {
                    // --- NEW: Split-pane layout for List mode ---
                    let main_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                        .split(f.area());

                    // --- Left Pane: Email List ---
                    let title = if app.is_loading { "Inbox (Loading...)" } else { "Primary Inbox" };
                    let header_cells = ["From", "Subject"]
                        .iter()
                        .map(|h| Cell::from(*h).style(Style::default().bold().underlined()));
                    let header = Row::new(header_cells).height(1);

                    let rows = app.emails.iter().enumerate().map(|(i, email)| {
                        let is_selected = i == app.selected_index;
                        let style = if is_selected {
                            Style::default().bg(Color::Yellow).fg(Color::Black)
                        } else if email.is_unread {
                            Style::default().bold().bg(Color::Red)
                        } else {
                            Style::default()
                        };
                        Row::new(vec![
                            Cell::from(email.from.clone()),
                            Cell::from(email.subject.clone()),
                        ])
                        .style(style)
                    });

                    let table = Table::new(rows, [Constraint::Percentage(40), Constraint::Percentage(60)])
                        .header(header)
                        .block(Block::default().borders(Borders::ALL).title(title));
                    f.render_widget(table, main_chunks[0]);

                    let selected_email_snippet = app.emails.get(app.selected_index)
                        .map_or(String::new(), |email| email.snippet.clone());

                    let preview = Paragraph::new(selected_email_snippet)
                        .block(Block::default().borders(Borders::ALL).title("Preview"))
                        .wrap(Wrap { trim: true });
                    f.render_widget(preview, main_chunks[1]);
                }
                AppMode::Viewing => {
                    let full_view_chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(0), Constraint::Length(1)])
                        .split(f.area());

                    let email_view = Paragraph::new(app.current_email_body.as_str())
                        .block(Block::default().borders(Borders::ALL).title("Email Content"))
                        .wrap(Wrap { trim: false })
                        .scroll((app.scroll_offset, 0));
                    f.render_widget(email_view, full_view_chunks[0]);
                }
            }

            let footer_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(f.area());

            let footer_text = match app.mode {
                AppMode::List => "↑/↓: Navigate  |  Enter: View Full Email  |  q: Quit",
                AppMode::Viewing => "↑/↓: Scroll  |  q: Back to List",
            };
            let footer = Paragraph::new(footer_text)
                .style(Style::default().fg(Color::Yellow).bg(Color::Black));
            f.render_widget(footer, footer_chunks[1]);
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app.mode {
                    AppMode::List => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Down => app.next(),
                        KeyCode::Up => app.previous(),
                        KeyCode::Enter => {
                            if let Some(selected_email) = app.emails.get(app.selected_index) {
                                let detail = google_api::get_full_message(&token, &selected_email.id).await?;
                                app.current_email_body = google_api::decode_email_body(&detail);
                                app.scroll_offset = 0;
                                app.mode = AppMode::Viewing;
                            }
                        }
                        _ => {}
                    },
                    AppMode::Viewing => match key.code {
                        KeyCode::Char('q') => app.mode = AppMode::List,
                        KeyCode::Down => app.scroll_down(),
                        KeyCode::Up => app.scroll_up(),
                        _ => {}
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}