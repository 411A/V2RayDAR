use std::{net::SocketAddr, time::Instant};

use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MenuView {
    Main,
    Subscriptions,
    NewSubscription,
    SubscriptionActions,
    Configurations,
    Logs,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MainItem {
    OpenConfig,
    Sharing,
    Proxy,
    Subscriptions,
    CleanCache,
    Configurations,
    Logs,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SubscriptionAction {
    EditName,
    EditUrl,
    EditPriority,
    Toggle,
    Delete,
    Back,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ConfigKey {
    Bind,
    TopN,
    RefreshSeconds,
    EncodedSubscription,
    PrioritizeStability,
    ReturnConfigsAsap,
    ScanAllConfigs,
    FetchTimeout,
    FetchConcurrency,
    MaxSubscriptionBytes,
    UseCacheOnly,
    EmergencyConfig,
    ProbeMode,
    SingBoxPath,
    ConnectTimeout,
    ActiveTimeout,
    StartupTimeout,
    ProbeConcurrency,
    ProbeBatchSize,
    ProbeProcessConcurrency,
    TestUrl,
    AcceptedStatuses,
    DownloadUrl,
    DownloadLimit,
    CleanOfflineDays,
    TokenRequired,
    Token,
    ProxyEnabled,
    ProxyPort,
    ProxyDiscoverable,
    ProxyHealthCheckUrl,
    ProxyHealthCheckInterval,
    ResetDefaults,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Action {
    Add,
    Toggle,
    Delete,
    EditName,
    EditUrl,
    EditPriority,
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    None,
    Command,
    NewSubscription(NewSubscriptionStep),
    Name,
    Url,
    Priority,
    ConfigValue(ConfigKey),
    ResetConfirm,
    CleanCacheConfirm,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum NewSubscriptionStep {
    Url,
    Name,
    Priority,
    Enabled,
}

#[derive(Debug, Clone)]
pub struct SubscriptionDraft {
    pub name: String,
    pub url: String,
    pub priority: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_field_names)]
pub struct HitMap {
    pub main_rows: Vec<(usize, ratatui::layout::Rect)>,
    pub subscription_rows: Vec<(usize, ratatui::layout::Rect)>,
    pub config_rows: Vec<(usize, ratatui::layout::Rect)>,
    pub logs_area: Option<ratatui::layout::Rect>,
    pub found_area: Option<ratatui::layout::Rect>,
    pub live_logs_area: Option<ratatui::layout::Rect>,
}

#[derive(Debug, Clone, Default)]
pub struct ScrollState {
    pub logs: usize,
    pub found: usize,
}

#[derive(Debug, Clone)]
pub struct TuiState {
    pub started_at: Instant,
    pub editable: AppConfig,
    pub active_bind: SocketAddr,
    pub view: MenuView,
    pub selected_main: usize,
    pub selected_subscription: usize,
    pub selected_action: usize,
    pub selected_config: usize,
    pub selected_log: usize,
    pub input_mode: InputMode,
    pub input: String,
    pub new_subscription: Option<SubscriptionDraft>,
    pub reset_code: Option<String>,
    pub status: String,
    pub dirty: bool,
    pub hits: HitMap,
    pub scroll: ScrollState,
}

impl TuiState {
    pub fn new(config: AppConfig) -> Self {
        let active_bind = config.bind;
        Self {
            started_at: Instant::now(),
            editable: config,
            active_bind,
            view: MenuView::Main,
            selected_main: 0,
            selected_subscription: 0,
            selected_action: 0,
            selected_config: 0,
            selected_log: 0,
            input_mode: InputMode::None,
            input: String::new(),
            new_subscription: None,
            reset_code: None,
            status: "Ready".to_string(),
            dirty: false,
            hits: HitMap::default(),
            scroll: ScrollState::default(),
        }
    }

    pub fn selected_subscription_mut(&mut self) -> Option<&mut crate::config::SubscriptionSource> {
        let index = self.selected_subscription_index()?;
        self.editable.subscriptions.get_mut(index)
    }

    pub fn selected_subscription_ref(&self) -> Option<&crate::config::SubscriptionSource> {
        self.editable
            .subscriptions
            .get(self.selected_subscription_index()?)
    }

    pub fn selected_subscription_index(&self) -> Option<usize> {
        self.editable
            .subscriptions
            .get(self.selected_subscription.checked_sub(1)?)
            .map(|_| self.selected_subscription - 1)
    }

    pub fn clamp_selection(&mut self) {
        if self.editable.subscriptions.is_empty() {
            self.selected_subscription = 0;
            return;
        }

        self.selected_subscription = self
            .selected_subscription
            .min(self.editable.subscriptions.len());
    }

    pub fn next_subscription_priority(&self) -> u32 {
        self.editable
            .subscriptions
            .iter()
            .map(|source| source.priority)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
}
