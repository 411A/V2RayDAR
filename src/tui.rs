use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use tokio::sync::RwLock;

use crate::{
    config::{AppConfig, SETTING_GUIDES},
    model::{RuntimeConfig, RuntimeState},
    paths::AppPaths,
};

#[derive(Debug, Clone, Copy)]
enum View {
    Dashboard,
    Settings,
    Paths,
    Endpoints,
}

impl View {
    const ALL: [Self; 4] = [
        Self::Dashboard,
        Self::Settings,
        Self::Paths,
        Self::Endpoints,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Settings => "Settings",
            Self::Paths => "Paths",
            Self::Endpoints => "Endpoints",
        }
    }
}

pub async fn run(
    initial_config: AppConfig,
    paths: AppPaths,
    state: Arc<RwLock<RuntimeState>>,
    runtime_config: Arc<RwLock<RuntimeConfig>>,
) -> Result<()> {
    let mut terminal = ratatui::try_init()?;
    let mut selected = 0_usize;
    let mut last_tick = Instant::now();

    let result = loop {
        let runtime = state.read().await.clone();
        let config = runtime_config.read().await.clone();

        terminal.draw(|frame| draw(frame, &initial_config, &config, &runtime, &paths, selected))?;

        let timeout = Duration::from_millis(250).saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(View::ALL.len() - 1);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Tab => {
                    selected = (selected + 1) % View::ALL.len();
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = Instant::now();
        }
    };

    ratatui::restore();
    result
}

fn draw(
    frame: &mut Frame<'_>,
    initial_config: &AppConfig,
    config: &RuntimeConfig,
    runtime: &RuntimeState,
    paths: &AppPaths,
    selected: usize,
) {
    let area = frame.area();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);

    frame.render_widget(header_widget(config), header);

    let [nav, content] =
        Layout::horizontal([Constraint::Length(18), Constraint::Fill(1)]).areas(body);
    frame.render_widget(nav_widget(selected), nav);

    match View::ALL[selected] {
        View::Dashboard => frame.render_widget(dashboard_widget(config, runtime), content),
        View::Settings => frame.render_widget(settings_widget(initial_config, config), content),
        View::Paths => frame.render_widget(paths_widget(paths), content),
        View::Endpoints => frame.render_widget(endpoints_widget(initial_config, config), content),
    }

    frame.render_widget(
        Paragraph::new("Up/Down or j/k navigate  Tab next  q quit")
            .style(Style::default().fg(Color::DarkGray)),
        footer,
    );
}

fn header_widget(config: &RuntimeConfig) -> Paragraph<'static> {
    let sharing = if config.sharing_enabled {
        if config.require_token {
            "LAN token"
        } else {
            "LAN open"
        }
    } else {
        "local only"
    };

    Paragraph::new(Line::from(vec![
        Span::styled(
            "V2RayDAR",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(sharing, Style::default().fg(Color::Yellow)),
        Span::raw(format!(
            "  top {}  refresh {}s",
            config.top_n, config.refresh_seconds
        )),
    ]))
    .block(Block::default().borders(Borders::BOTTOM))
}

fn nav_widget(selected: usize) -> List<'static> {
    let items = View::ALL
        .iter()
        .enumerate()
        .map(|(index, view)| {
            let label = if index == selected {
                format!("> {}", view.title())
            } else {
                format!("  {}", view.title())
            };
            ListItem::new(label)
        })
        .collect::<Vec<_>>();

    List::new(items).block(Block::default().borders(Borders::RIGHT).title("Menu"))
}

fn dashboard_widget(config: &RuntimeConfig, runtime: &RuntimeState) -> Paragraph<'static> {
    let lines = vec![
        Line::from(format!("Bind: {}", config.bind)),
        Line::from(format!("Probe: {}", config.probe_mode)),
        Line::from(format!(
            "Last refresh: {}",
            runtime.last_refresh.as_deref().unwrap_or("never")
        )),
        Line::from(format!(
            "Candidates: {} total, {} reachable",
            runtime.total_candidates, runtime.reachable_candidates
        )),
        Line::from(format!("Fetch errors: {}", runtime.fetch_errors.len())),
        Line::from(""),
        Line::from("Top reachable configs:"),
    ]
    .into_iter()
    .chain(
        runtime
            .ranked
            .iter()
            .filter(|item| item.reachable)
            .take(config.top_n)
            .map(|item| {
                Line::from(format!(
                    "#{:<3} {:<8} {:<24} {} ms",
                    item.rank,
                    item.protocol,
                    item.name,
                    item.latency_ms.unwrap_or_default()
                ))
            }),
    )
    .collect::<Vec<_>>();

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Dashboard"))
        .wrap(Wrap { trim: true })
}

fn settings_widget(initial_config: &AppConfig, config: &RuntimeConfig) -> Paragraph<'static> {
    let mut lines = vec![
        Line::from(format!("Listen address: {}", config.bind)),
        Line::from(format!("LAN sharing: {}", on_off(config.sharing_enabled))),
        Line::from(format!("URL token: {}", on_off(config.require_token))),
        Line::from(format!(
            "Encoded feed: {}",
            on_off(config.encoded_subscription)
        )),
        Line::from(format!("Probe mode: {}", config.probe_mode)),
        Line::from(format!("sing-box: {}", initial_config.probe.sing_box_path)),
        Line::from(""),
    ];

    lines.extend(SETTING_GUIDES.iter().map(|guide| {
        Line::from(vec![
            Span::styled(
                format!("{:<18}", guide.label),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(guide.help),
        ])
    }));

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Settings"))
        .wrap(Wrap { trim: true })
}

fn paths_widget(paths: &AppPaths) -> Paragraph<'static> {
    let lines = vec![
        Line::from(format!(
            "Mode: {}",
            if paths.portable {
                "portable"
            } else {
                "installed"
            }
        )),
        Line::from(format!("App folder: {}", paths.root_dir.display())),
        Line::from(format!("Config: {}", paths.config_path.display())),
        Line::from(format!(
            "Subscriptions: {}",
            paths.subscriptions_dir.display()
        )),
        Line::from(format!("Cache: {}", paths.cache_dir.display())),
        Line::from(format!("Logs: {}", paths.logs_dir.display())),
        Line::from(format!("Backups: {}", paths.backups_dir.display())),
        Line::from(format!("Runtime: {}", paths.runtime_dir.display())),
    ];

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Paths"))
        .wrap(Wrap { trim: true })
}

fn endpoints_widget(initial_config: &AppConfig, config: &RuntimeConfig) -> Paragraph<'static> {
    let local = initial_config.subscription_url("127.0.0.1", false);
    let raw = initial_config.subscription_url("127.0.0.1", true);
    let lan_hint = if config.require_token {
        format!(
            "http://LAN_IP:{}/subscription?token={}",
            config.bind.port(),
            config.token
        )
    } else {
        format!("http://LAN_IP:{}/subscription", config.bind.port())
    };

    let lines = vec![
        Line::from(format!("Local subscription: {local}")),
        Line::from(format!("Local raw feed: {raw}")),
        Line::from(format!(
            "Health: http://127.0.0.1:{}/health",
            config.bind.port()
        )),
        Line::from(""),
        Line::from(format!(
            "LAN status: {}",
            if config.sharing_enabled {
                "enabled"
            } else {
                "disabled"
            }
        )),
        Line::from(format!("LAN URL: {lan_hint}")),
    ];

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Endpoints"))
        .wrap(Wrap { trim: true })
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

#[allow(dead_code)]
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ])
    .split(vertical[1]);
    horizontal[1]
}
