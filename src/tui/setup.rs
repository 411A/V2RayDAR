use anyhow::{Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::path::Path;

use crate::{
    config::AppConfig,
    constants::{TUI_SETUP_POLL_INTERVAL, sing_box_download_url},
    paths::AppPaths,
    sing_box,
};

use super::util::save_config;

pub async fn run(config: &mut AppConfig, paths: &AppPaths) -> Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut terminal = ratatui::try_init()?;
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;

    let mut state = SetupState {
        input: config.probe.sing_box_path.clone(),
        status: "Enter the sing-box path or PATH command, then press Enter".to_string(),
        verifying: false,
    };

    let result = loop {
        terminal.draw(|frame| draw(frame, &state, paths))?;

        if !event::poll(TUI_SETUP_POLL_INTERVAL).unwrap_or(false) {
            continue;
        }

        match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    break Err(anyhow!("sing-box setup cancelled"));
                }

                match key.code {
                    KeyCode::Esc => break Err(anyhow!("sing-box setup cancelled")),
                    KeyCode::Enter => {
                        let candidate = sing_box::normalize_path(&state.input);
                        state.verifying = true;
                        state.status = "Verifying sing-box path...".to_string();
                        terminal.draw(|frame| draw(frame, &state, paths))?;

                        match sing_box::verify_path(&candidate).await {
                            Ok(()) => {
                                config.probe.sing_box_path = candidate;
                                config.probe.sing_box_path_auto = false;
                                save_config(&paths.config_path, config)?;
                                break Ok(());
                            }
                            Err(error) => {
                                state.verifying = false;
                                state.status = error.to_string();
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        state.input.pop();
                    }
                    KeyCode::Char(value) => {
                        state.input.push(value);
                    }
                    _ => {}
                }
            }
            Ok(_) => {}
            Err(error) => break Err(error.into()),
        }
    };

    super::restore_terminal().and(result)
}

struct SetupState {
    input: String,
    status: String,
    verifying: bool,
}

fn draw(frame: &mut Frame<'_>, state: &SetupState, paths: &AppPaths) {
    let area = frame.area();
    let [top, body, footer] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .areas(area);

    let title = Paragraph::new(vec![
        Line::from(Span::styled(
            "V2RayDAR First Run",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("Active probing needs a local sing-box executable path."),
    ])
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(title, top);

    let display_input = if state.verifying {
        state.input.clone()
    } else {
        format!("{}_", state.input)
    };
    let guide = sing_box::setup_guide();
    let mut lines = vec![
        Line::from(format!("Detected OS: {}", guide.platform)),
        Line::from(format!(
            "Recommended sing-box version: v{}",
            sing_box::recommended_version()
        )),
        Line::from(format!("Use executable: {}", guide.executable_name)),
        Line::from(format!("Download asset: {}", guide.release_asset)),
        Line::from("Download recommended release:"),
        Line::from(Span::styled(
            sing_box_download_url(),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from("Examples:"),
    ];
    lines.extend(guide.example_paths.iter().map(|path| {
        Line::from(Span::styled(
            format!("  {path}"),
            Style::default().fg(Color::Green),
        ))
    }));
    lines.extend([Line::from(""), Line::from("Notes:")]);
    lines.extend(
        guide
            .notes
            .iter()
            .map(|note| Line::from(format!("  {note}"))),
    );
    lines.extend([
        Line::from(""),
        Line::from("Config will be saved to:"),
        Line::from(Span::styled(
            display_path(&paths.config_path),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from("sing-box path:"),
        Line::from(Span::styled(
            display_input,
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            state.status.clone(),
            Style::default().fg(if state.verifying {
                Color::Yellow
            } else {
                Color::White
            }),
        )),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Setup"))
            .wrap(Wrap { trim: true }),
        body,
    );

    frame.render_widget(
        Paragraph::new("Enter verify/save | Esc cancel | Ctrl+C quit"),
        footer,
    );
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}
