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
    fn previous(&mut self, body_request_tx: mpsc::Sender<String>) {
        if !self.emails.is_empty() {
            let new_index = if self.selected_index > 0 {
                self.selected_index - 1
            } else {
                self.emails.len() - 1
            };
            self.select(new_index, body_request_tx);
        }
    }

    fn next(&mut self, body_request_tx: mpsc::Sender<String>) {
        if !self.emails.is_empty() {
            let new_index = if self.selected_index < self.emails.len() - 1 {
                self.selected_index + 1
            } else {
                0
            };
            self.select(new_index, body_request_tx);
        }
    }

    fn select(&mut self, index: usize, body_request_tx: mpsc::Sender<String>) {
        if self.selected_index != index || self.current_email_body.is_empty() {
            self.selected_index = index;
            self.scroll_offset = 0;
            self.current_email_body = "Loading...".to_string();
            if let Some(email) = self.emails.get(index) {
                let _ = body_request_tx.try_send(email.id.clone());
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
    // --- Channel Setup ---
    let (header_tx, mut header_rx) = mpsc::channel::<EmailInfo>(100);
    let (body_request_tx, mut body_request_rx) = mpsc::channel::<String>(10);
    let (body_result_tx, mut body_result_rx) = mpsc::channel::<String>(10);

    // --- Background Tasks ---
    let token_clone_1 = token.clone();
    tokio::spawn(async move {
        if let Ok(message_list) = google_api::list_messages(&token_clone_1).await {
            let message_ids = message_list.messages.unwrap_or_default();
            let header_futures = message_ids
                .iter()
                .map(|msg| google_api::get_message_headers(&token_clone_1, &msg.id));
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
                    if header_tx.send(email_info).await.is_err() { break; }
                }
            }
        }
    });

    let token_clone_2 = token.clone();
    tokio::spawn(async move {
        while let Some(email_id) = body_request_rx.recv().await {
            if let Ok(detail) = google_api::get_full_message(&token_clone_2, &email_id).await {
                let body = google_api::decode_email_body(&detail);
                if body_result_tx.send(body).await.is_err() { break; }
            }
        }
    });

    // --- App Initialization ---
    let mut app = App {
        mode: AppMode::List,
        is_loading: true,
        emails: Vec::new(),
        selected_index: 0,
        current_email_body: "Loading email list...".to_string(),
        scroll_offset: 0,
    };

    // --- TUI Setup ---
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // --- Main Loop ---
    let mut initial_load_done = false;
    loop {
        // --- Event & Data Handling ---
        if !app.is_loading {
             if let Ok(body) = body_result_rx.try_recv() {
                app.current_email_body = body;
            }
        } else {
            match header_rx.try_recv() {
                Ok(email) => {
                    app.emails.push(email);
                    if !initial_load_done {
                        app.select(0, body_request_tx.clone());
                        initial_load_done = true;
                    }
                },
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    app.is_loading = false;
                },
                _ => {}
            }
        }
        
        // --- Drawing ---
        terminal.draw(|f| {
            let main_area = f.area();
            
            match app.mode {
                AppMode::List => {
                    let main_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
                        .split(main_area);
                    
                    let title = if app.is_loading { "Inbox (Loading...)" } else { "Primary Inbox" };
                    let header_cells = ["From", "Subject"]
                        .iter()
                        .map(|h| Cell::from(*h).style(Style::default().bold().underlined()));
                    let header = Row::new(header_cells).height(1);

                    let rows = app.emails.iter().enumerate().map(|(i, email)| {
                        let is_selected = i == app.selected_index;
                        let style = if is_selected {
                            Style::default().bg(Color::Blue).fg(Color::White)
                        } else if email.is_unread {
                            Style::default().bold().bg(Color::DarkGray)
                        } else { Style::default() };
                        Row::new(vec![
                            Cell::from(email.from.clone()),
                            Cell::from(email.subject.clone()),
                        ]).style(style)
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
                    let email_view = Paragraph::new(app.current_email_body.as_str())
                        .block(Block::default().borders(Borders::ALL).title("Content"))
                        .wrap(Wrap { trim: false })
                        .scroll((app.scroll_offset, 0));
                    f.render_widget(email_view, main_area);
                }
            }

            let footer_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(main_area);

            let footer_text = match app.mode {
                AppMode::List => "↑/↓: Navigate | Enter: View Full Email | q: Quit",
                AppMode::Viewing => "↑/↓: Scroll | q: Back to List",
            };
            let footer = Paragraph::new(footer_text)
                .style(Style::default().fg(Color::White).bg(Color::DarkGray));
            f.render_widget(footer, footer_chunks[1]);
        })?;

        // --- User Input ---
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match app.mode {
                    AppMode::List => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Down => app.next(body_request_tx.clone()),
                        KeyCode::Up => app.previous(body_request_tx.clone()),
                        KeyCode::Enter => {
                            app.mode = AppMode::Viewing;
                        }
                        _ => {}
                    },
                    AppMode::Viewing => match key.code {
                        KeyCode::Char('q') => {
                            if let Some(email) = app.emails.get_mut(app.selected_index) {
                                if email.is_unread {
                                    if google_api::mark_as_read(&token, &email.id).await.is_ok() {
                                        email.is_unread = false;
                                    }
                                }
                            }
                            app.mode = AppMode::List;
                        }
                        KeyCode::Down => app.scroll_down(),
                        KeyCode::Up => app.scroll_up(),
                        _ => {}
                    }
                }
            }
        }
    }

    // --- Cleanup ---
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}