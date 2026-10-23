//! GoldWatcher desktop application.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike, Utc};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, Pos2, Rect,
    RichText, Sense, Shape, Stroke, TextureOptions, Vec2,
};

use crate::alerts;
use crate::data::{FeedEvent, FeedSnapshot, FeedWorker, HistoryPayload};
use crate::model::{
    beijing_tz, fmt_num, fmt_pct, format_threshold, now_beijing, Alert, AlertMetric, AlertScope,
    AppConfig, ChartWindow, Direction, FxRates, HistoryStore, MarketId, MarketMetrics, PricePoint,
    Quote, SessionAlert, SessionId,
};
use crate::notify;
use crate::storage::{self, Paths};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Alerts,
    Sessions,
    Events,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutMode {
    ThreeColumn,
    TwoColumn,
    Stacked,
}

impl LayoutMode {
    fn for_size(width: f32, height: f32) -> Self {
        let ratio = width / height.max(1.0);
        if height < 680.0 {
            return Self::Stacked;
        }
        if width >= 1250.0 && ratio >= 1.12 {
            Self::ThreeColumn
        } else if width >= 980.0 && ratio >= 0.90 {
            Self::TwoColumn
        } else {
            Self::Stacked
        }
    }
}

#[derive(Debug, Clone)]
struct Toast {
    event: crate::model::AlertEvent,
    expires: Instant,
}

struct CaptureState {
    path: std::path::PathBuf,
    started: Instant,
    delay: Duration,
    requested: bool,
}

struct FeedState {
    online: bool,
    last_attempt: Option<DateTime<Local>>,
    last_attempt_instant: Option<Instant>,
    last_success: Option<DateTime<Local>>,
    latency_ms: u128,
    source: String,
    warning: Option<String>,
}

impl Default for FeedState {
    fn default() -> Self {
        Self {
            online: false,
            last_attempt: None,
            last_attempt_instant: None,
            last_success: None,
            latency_ms: 0,
            source: "正在连接…".to_owned(),
            warning: None,
        }
    }
}

#[derive(Debug, Clone)]
struct SessionEditor {
    edit_id: Option<String>,
    name: String,
    session: SessionId,
    watch_open: bool,
    watch_close: bool,
    enabled: bool,
    notify: bool,
    flash: bool,
    error: Option<String>,
}

impl SessionEditor {
    fn new() -> Self {
        Self {
            edit_id: None,
            name: String::new(),
            session: SessionId::NewYork,
            watch_open: true,
            watch_close: true,
            enabled: true,
            notify: true,
            flash: true,
            error: None,
        }
    }

    fn from_alert(alert: &SessionAlert) -> Self {
        Self {
            edit_id: Some(alert.id.clone()),
            name: alert.name.clone(),
            session: alert.session,
            watch_open: alert.watch_open,
            watch_close: alert.watch_close,
            enabled: alert.enabled,
            notify: alert.notify,
            flash: alert.flash,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
struct AlertEditor {
    edit_id: Option<String>,
    name: String,
    scope: AlertScope,
    metric: AlertMetric,
    direction: Direction,
    threshold: f64,
    enabled: bool,
    repeat: bool,
    cooldown_secs: u64,
    notify: bool,
    flash: bool,
    error: Option<String>,
}

impl AlertEditor {
    fn new(scope: AlertScope, quotes: &BTreeMap<MarketId, Quote>, metrics: &MarketMetrics) -> Self {
        let threshold = default_threshold(scope, quotes, metrics);
        Self {
            edit_id: None,
            name: String::new(),
            scope,
            metric: AlertMetric::Price,
            direction: Direction::Above,
            threshold,
            enabled: true,
            repeat: false,
            cooldown_secs: 300,
            notify: true,
            flash: true,
            error: None,
        }
    }

    fn from_alert(alert: &Alert) -> Self {
        Self {
            edit_id: Some(alert.id.clone()),
            name: alert.name.clone(),
            scope: alert.scope,
            metric: alert.metric,
            direction: alert.direction,
            threshold: alert.threshold,
            enabled: alert.enabled,
            repeat: alert.repeat,
            cooldown_secs: alert.cooldown_secs,
            notify: alert.notify,
            flash: alert.flash,
            error: None,
        }
    }
}

fn default_threshold(
    scope: AlertScope,
    quotes: &BTreeMap<MarketId, Quote>,
    metrics: &MarketMetrics,
) -> f64 {
    match scope {
        AlertScope::Market { market } => match quotes.get(&market) {
            Some(q) if q.is_valid() => round_to(q.price * 1.01, market.decimals().max(2)),
            _ => 4_200.0,
        },
        AlertScope::ComexBasis => {
            let basis = metrics.comex_basis.unwrap_or(5.0);
            round_to(if basis.abs() < 0.01 { 5.0 } else { basis }, 2)
        }
        AlertScope::SgePremium => {
            let p = metrics.sge_premium_pct.unwrap_or(1.0);
            round_to(if p.abs() < 0.01 { 1.0 } else { p }, 2)
        }
        AlertScope::UsdCny => metrics.usd_cny.map(|v| round_to(v, 4)).unwrap_or(6.7),
    }
}

fn round_to(value: f64, decimals: usize) -> f64 {
    let factor = 10f64.powi(decimals as i32);
    (value * factor).round() / factor
}

#[derive(Debug, Clone)]
enum AlertAction {
    Toggle(String, bool),
    Edit(String),
    AskDelete(String),
    CancelDelete,
    Delete(String),
}

#[derive(Debug, Clone)]
enum SessionAction {
    Toggle(String, bool),
    Edit(String),
    AskDelete(String),
    CancelDelete,
    Delete(String),
}

pub struct GoldWatcherApp {
    paths: Paths,
    config: AppConfig,
    quotes: BTreeMap<MarketId, Quote>,
    fx: Option<FxRates>,
    metrics: MarketMetrics,
    history: HistoryStore,
    feed: FeedState,
    worker: FeedWorker,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    tray: Option<crate::tray::TrayHandle>,
    tray_available: bool,
    requested_history: BTreeSet<MarketId>,
    loading_history: BTreeSet<MarketId>,
    history_notice: Option<String>,
    logo: egui::TextureHandle,
    show_settings: bool,
    editor: Option<AlertEditor>,
    session_editor: Option<SessionEditor>,
    right_tab: RightTab,
    delete_confirm: Option<String>,
    clear_events_confirm: bool,
    toasts: Vec<Toast>,
    settings_error: Option<String>,
    last_save: Instant,
    last_history_save: Instant,
    last_session_check: Instant,
    session_states: HashMap<SessionId, bool>,
    layout_mode: LayoutMode,
    show_alert_drawer: bool,
    capture: Option<CaptureState>,
    dirty: bool,
}

impl GoldWatcherApp {
    pub fn new(cc: &eframe::CreationContext<'_>, icon: egui::IconData) -> Self {
        theme::install(&cc.egui_ctx);

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        );
        let logo = cc
            .egui_ctx
            .load_texture("goldwatcher-logo", image, TextureOptions::LINEAR);

        let paths = Paths::discover();
        let mut config = storage::load_config(&paths);
        sanitize_config(&mut config);
        let mut history = storage::load_history(&paths);
        history.data.retain(|_, points| {
            points.retain(|p| p.price.is_finite() && p.price > 0.0);
            !points.is_empty()
        });
        alerts::reset_conditions(&mut config.alerts);

        if storage::autostart_supported() {
            config.auto_start = storage::autostart_enabled();
        }

        let worker = FeedWorker::spawn(config.poll_secs);
        worker.fetch_history(config.selected_market);

        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let tray = crate::tray::spawn(icon.rgba.clone(), icon.width, icon.height);
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let tray = None;
        let tray_available = {
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            {
                tray.is_some()
            }
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            {
                false
            }
        };

        let capture_delay = std::env::var("GOLDWATCHER_CAPTURE_DELAY")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(7);
        let capture = std::env::var("GOLDWATCHER_CAPTURE")
            .ok()
            .map(|path| CaptureState {
                path: path.into(),
                started: Instant::now(),
                delay: Duration::from_secs(capture_delay.max(2)),
                requested: false,
            });

        let now = now_beijing();
        let session_states = SessionId::ALL
            .into_iter()
            .map(|session| (session, session.is_open(now)))
            .collect();

        let mut requested_history = BTreeSet::new();
        if matches!(
            config.selected_market,
            MarketId::London | MarketId::NewYork | MarketId::Shanghai
        ) {
            requested_history.insert(config.selected_market);
        }

        Self {
            paths,
            config,
            quotes: BTreeMap::new(),
            fx: None,
            metrics: MarketMetrics::default(),
            history,
            feed: FeedState::default(),
            worker,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            tray,
            tray_available,
            requested_history,
            loading_history: BTreeSet::new(),
            history_notice: None,
            logo,
            show_settings: false,
            editor: None,
            session_editor: None,
            right_tab: RightTab::Alerts,
            delete_confirm: None,
            clear_events_confirm: false,
            toasts: Vec::new(),
            settings_error: None,
            last_save: Instant::now(),
            last_history_save: Instant::now(),
            last_session_check: Instant::now(),
            session_states,
            layout_mode: LayoutMode::ThreeColumn,
            show_alert_drawer: false,
            capture,
            dirty: false,
        }
    }

    fn selected_quote(&self) -> Option<&Quote> {
        self.quotes.get(&self.config.selected_market)
    }

    fn ensure_history(&mut self, market: MarketId) {
        if matches!(
            market,
            MarketId::London | MarketId::NewYork | MarketId::Shanghai
        ) && !self.requested_history.contains(&market)
            && !self.loading_history.contains(&market)
        {
            self.loading_history.insert(market);
            self.worker.fetch_history(market);
        }
    }

    fn pump_events(&mut self, ctx: &egui::Context) {
        loop {
            let event = match self.worker.events.try_recv() {
                Ok(event) => event,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            };
            match event {
                FeedEvent::Snapshot(snapshot) => self.apply_snapshot(ctx, snapshot),
                FeedEvent::History(payload) => self.apply_history(payload),
            }
        }
    }

    fn apply_snapshot(&mut self, ctx: &egui::Context, snapshot: FeedSnapshot) {
        if std::env::var_os("GOLDWATCHER_DEBUG").is_some() {
            eprintln!(
                "[feed] quotes={} fx={} latency={}ms warning={:?}",
                snapshot.quotes.len(),
                snapshot.fx.is_some(),
                snapshot.latency_ms,
                snapshot.warning
            );
        }
        self.feed.last_attempt = Some(snapshot.fetched_at);
        self.feed.last_attempt_instant = Some(Instant::now());
        self.feed.latency_ms = snapshot.latency_ms;
        self.feed.source = snapshot.source;
        self.feed.warning = snapshot.warning;
        if snapshot.quotes.is_empty() {
            self.feed.online = false;
        } else {
            self.feed.online = true;
            self.feed.last_success = Some(snapshot.fetched_at);
        }

        let had_new_quotes = !snapshot.quotes.is_empty();
        let now_ts = Utc::now().timestamp();
        for (market, mut quote) in snapshot.quotes {
            if let Some(old) = self.quotes.get(&market) {
                if !quote.prev_close.is_finite() || quote.prev_close <= 0.0 {
                    quote.prev_close = old.prev_close;
                }
                if !quote.open.is_finite() || quote.open <= 0.0 {
                    quote.open = old.open;
                }
                if !quote.high.is_finite() || quote.high <= 0.0 {
                    quote.high = old.high;
                }
                if !quote.low.is_finite() || quote.low <= 0.0 {
                    quote.low = old.low;
                }
            }
            self.history
                .push(market, PricePoint::new(now_ts, quote.price));
            self.quotes.insert(market, quote);
        }
        if let Some(fx) = snapshot.fx {
            if fx.is_valid() {
                self.fx = Some(fx);
            }
        }
        self.metrics = MarketMetrics::derive(&self.quotes, self.fx.as_ref());

        if had_new_quotes {
            let max_age = (self.config.poll_secs as i64 * 3).max(60);
            let fresh_quotes: BTreeMap<MarketId, Quote> = self
                .quotes
                .iter()
                .filter(|(_, quote)| (Local::now() - quote.updated_at).num_seconds() <= max_age)
                .map(|(market, quote)| (*market, quote.clone()))
                .collect();
            if !fresh_quotes.is_empty() {
                let fresh_metrics = MarketMetrics::derive(&fresh_quotes, self.fx.as_ref());
                let result = alerts::evaluate(
                    &mut self.config.alerts,
                    &fresh_quotes,
                    &fresh_metrics,
                    Local::now(),
                );
                if result.changed {
                    self.dirty = true;
                }
                for event in result.events {
                    let source_alert = self
                        .config
                        .alerts
                        .iter()
                        .find(|alert| alert.id == event.alert_id)
                        .cloned();
                    let (notify, flash) = source_alert
                        .as_ref()
                        .map(|alert| (alert.notify, alert.flash))
                        .unwrap_or((true, true));
                    self.record_alert_event(ctx, event, notify, flash);
                }
            }
        }

        if self.selected_quote().is_some() {
            self.ensure_history(self.config.selected_market);
        }
    }

    fn record_alert_event(
        &mut self,
        ctx: &egui::Context,
        event: crate::model::AlertEvent,
        notify: bool,
        flash: bool,
    ) {
        if self.config.notifications && notify {
            notify::notify(
                format!("金瞳 GoldWatcher · {}", event.title),
                event.message.clone(),
                false,
            );
        }
        if self.config.flash && flash {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
        }
        self.config.events.insert(0, event.clone());
        self.config.events.truncate(100);
        self.toasts.push(Toast {
            event,
            expires: Instant::now() + Duration::from_secs(10),
        });
        if self.toasts.len() > 3 {
            let drain = self.toasts.len() - 3;
            self.toasts.drain(0..drain);
        }
        self.dirty = true;
    }

    fn check_session_events(&mut self, ctx: &egui::Context) {
        if self.last_session_check.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.last_session_check = Instant::now();
        let now = now_beijing();
        let mut fired: Vec<(crate::model::AlertEvent, bool, bool)> = Vec::new();
        for session in SessionId::ALL {
            let open = session.is_open(now);
            let Some(previous) = self.session_states.insert(session, open) else {
                continue;
            };
            if previous == open {
                continue;
            }
            let events = alerts::evaluate_session(&mut self.config.session_alerts, session, open);
            fired.extend(events);
        }
        for (event, notify, flash) in fired {
            self.record_alert_event(ctx, event, notify, flash);
        }
    }

    fn total_active_alerts(&self) -> usize {
        alerts::active_count(&self.config.alerts)
            + self
                .config
                .session_alerts
                .iter()
                .filter(|alert| alert.enabled)
                .count()
    }

    fn open_session_editor(&mut self) {
        self.session_editor = Some(SessionEditor::new());
    }

    fn commit_session_editor(&mut self, editor: SessionEditor) {
        if !editor.watch_open && !editor.watch_close {
            return;
        }
        let events = match (editor.watch_open, editor.watch_close) {
            (true, true) => "开盘 / 收盘",
            (true, false) => "开盘",
            (false, true) => "收盘",
            (false, false) => return,
        };
        let name = if editor.name.trim().is_empty() {
            format!("{} {}", editor.session.name(), events)
        } else {
            editor.name.trim().to_owned()
        };
        if let Some(id) = editor.edit_id {
            if let Some(alert) = self
                .config
                .session_alerts
                .iter_mut()
                .find(|alert| alert.id == id)
            {
                alert.name = name;
                alert.session = editor.session;
                alert.watch_open = editor.watch_open;
                alert.watch_close = editor.watch_close;
                alert.enabled = editor.enabled;
                alert.notify = editor.notify;
                alert.flash = editor.flash;
            }
        } else {
            let mut alert =
                SessionAlert::new(editor.session, editor.watch_open, editor.watch_close);
            alert.name = name;
            alert.enabled = editor.enabled;
            alert.notify = editor.notify;
            alert.flash = editor.flash;
            self.config.session_alerts.insert(0, alert);
        }
        self.dirty = true;
    }

    fn apply_session_action(&mut self, action: SessionAction) {
        match action {
            SessionAction::Toggle(id, enabled) => {
                if let Some(alert) = self
                    .config
                    .session_alerts
                    .iter_mut()
                    .find(|alert| alert.id == id)
                {
                    alert.enabled = enabled;
                    self.dirty = true;
                }
            }
            SessionAction::Edit(id) => {
                if let Some(alert) = self
                    .config
                    .session_alerts
                    .iter()
                    .find(|alert| alert.id == id)
                    .cloned()
                {
                    self.session_editor = Some(SessionEditor::from_alert(&alert));
                }
            }
            SessionAction::AskDelete(id) => self.delete_confirm = Some(id),
            SessionAction::CancelDelete => self.delete_confirm = None,
            SessionAction::Delete(id) => {
                self.config.session_alerts.retain(|alert| alert.id != id);
                self.delete_confirm = None;
                self.dirty = true;
            }
        }
    }

    fn apply_history(&mut self, payload: HistoryPayload) {
        self.loading_history.remove(&payload.market);
        self.requested_history.insert(payload.market);
        match payload.points {
            Ok(points) => {
                if points.is_empty() {
                    if self
                        .history
                        .data
                        .get(&payload.market)
                        .is_none_or(|v| v.len() < 2)
                    {
                        self.history_notice = Some(format!(
                            "{} 暂无可用历史分钟线，将使用本地采样走势",
                            payload.market.short_name()
                        ));
                    }
                } else {
                    self.history.seed(payload.market, points);
                    self.history_notice = None;
                }
            }
            Err(err) => {
                self.history_notice = Some(format!(
                    "{} 历史走势加载失败：{err}",
                    payload.market.short_name()
                ));
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn handle_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = &self.tray else {
            return;
        };
        for command in crate::tray::poll(tray) {
            match command {
                crate::tray::TrayCommand::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                crate::tray::TrayCommand::Refresh => {
                    self.worker.refresh_now();
                    self.feed.last_attempt_instant = Some(Instant::now());
                }
                crate::tray::TrayCommand::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    fn handle_tray(&mut self, _ctx: &egui::Context) {}

    fn maybe_save(&mut self) {
        if self.dirty && self.last_save.elapsed() >= Duration::from_millis(900) {
            match storage::save_config(&self.paths, &self.config) {
                Ok(()) => self.dirty = false,
                Err(err) => self.settings_error = Some(err),
            }
            self.last_save = Instant::now();
        }
        if self.last_history_save.elapsed() >= Duration::from_secs(300) {
            let _ = storage::save_history(&self.paths, &self.history);
            self.last_history_save = Instant::now();
        }
    }

    fn save_now(&mut self) {
        let _ = storage::save_config(&self.paths, &self.config);
        let _ = storage::save_history(&self.paths, &self.history);
    }

    fn quick_add(
        &mut self,
        scope: AlertScope,
        metric: AlertMetric,
        direction: Direction,
        threshold: f64,
        name: &str,
    ) {
        let mut alert = Alert::new(name, scope, metric, direction, threshold);
        alert.notify = self.config.notifications;
        alert.flash = self.config.flash;
        self.config.alerts.insert(0, alert);
        self.dirty = true;
    }

    fn open_quick_alert(&mut self) {
        let scope = AlertScope::Market {
            market: self.config.selected_market,
        };
        self.editor = Some(AlertEditor::new(scope, &self.quotes, &self.metrics));
    }

    fn apply_alert_action(&mut self, action: AlertAction) {
        match action {
            AlertAction::Toggle(id, enabled) => {
                if let Some(alert) = self.config.alerts.iter_mut().find(|a| a.id == id) {
                    alert.enabled = enabled;
                    if enabled {
                        alert.last_condition = false;
                    }
                    self.dirty = true;
                }
            }
            AlertAction::Edit(id) => {
                if let Some(alert) = self.config.alerts.iter().find(|a| a.id == id).cloned() {
                    self.editor = Some(AlertEditor::from_alert(&alert));
                }
            }
            AlertAction::AskDelete(id) => {
                self.delete_confirm = Some(id);
            }
            AlertAction::CancelDelete => {
                self.delete_confirm = None;
            }
            AlertAction::Delete(id) => {
                self.config.alerts.retain(|a| a.id != id);
                self.delete_confirm = None;
                self.dirty = true;
            }
        }
    }

    fn commit_editor(&mut self, editor: AlertEditor) {
        let threshold = if editor.threshold.is_finite() {
            editor.threshold
        } else {
            return;
        };
        let fallback_name = format!(
            "{} {} {}",
            editor.scope.short_label(),
            editor.metric_label_for_editor(),
            format_threshold(editor.scope, editor.metric, threshold)
        );
        let name = if editor.name.trim().is_empty() {
            fallback_name
        } else {
            editor.name.trim().to_owned()
        };
        if let Some(id) = editor.edit_id {
            if let Some(alert) = self.config.alerts.iter_mut().find(|a| a.id == id) {
                alert.name = name;
                alert.scope = editor.scope;
                alert.metric = editor.metric;
                alert.direction = editor.direction;
                alert.threshold = threshold;
                alert.enabled = editor.enabled;
                alert.repeat = editor.repeat;
                alert.cooldown_secs = editor.cooldown_secs.max(30);
                alert.notify = editor.notify;
                alert.flash = editor.flash;
                alert.last_condition = false;
            }
        } else {
            let mut alert = Alert::new(
                name,
                editor.scope,
                editor.metric,
                editor.direction,
                threshold,
            );
            alert.enabled = editor.enabled;
            alert.repeat = editor.repeat;
            alert.cooldown_secs = editor.cooldown_secs.max(30);
            alert.notify = editor.notify;
            alert.flash = editor.flash;
            self.config.alerts.insert(0, alert);
        }
        self.dirty = true;
    }
}

pub const POLL_OPTIONS: [u64; 4] = [15, 60, 120, 300];

fn normalize_poll_secs(secs: u64) -> u64 {
    if POLL_OPTIONS.contains(&secs) {
        secs
    } else {
        15
    }
}

fn sanitize_config(config: &mut AppConfig) {
    config.poll_secs = normalize_poll_secs(config.poll_secs);
    for alert in &mut config.alerts {
        if !alert.threshold.is_finite() {
            alert.threshold = 0.0;
        }
        alert.cooldown_secs = alert.cooldown_secs.clamp(30, 86_400);
    }
    for alert in &mut config.session_alerts {
        if !alert.watch_open && !alert.watch_close {
            alert.watch_open = true;
        }
    }
    config.events.truncate(100);
}

impl eframe::App for GoldWatcherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_events(ctx);
        self.check_session_events(ctx);
        self.handle_tray(ctx);
        self.maybe_save();
        self.capture_frame(ctx);

        let screen = ctx.screen_rect().size();
        self.layout_mode = LayoutMode::for_size(screen.x - 44.0, screen.y - 28.0);
        if self.layout_mode != LayoutMode::TwoColumn {
            self.show_alert_drawer = false;
        }

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BG)
                    .inner_margin(Margin::symmetric(22, 14)),
            )
            .show(ctx, |ui| {
                theme::ambient(ui);
                self.header(ui);
                ui.add_space(10.0);
                self.body(ui);
                self.status_bar(ui);
            });

        self.alert_drawer(ctx);
        self.alert_editor_modal(ctx);
        self.session_editor_modal(ctx);
        self.settings_modal(ctx);
        self.show_toasts(ctx);

        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_now();
    }
}

impl GoldWatcherApp {
    fn capture_frame(&mut self, ctx: &egui::Context) {
        let Some(capture) = &mut self.capture else {
            return;
        };
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = screenshot {
            let [width, height] = image.size;
            let mut bytes = Vec::with_capacity(width * height * 4);
            for pixel in &image.pixels {
                bytes.extend_from_slice(&pixel.to_array());
            }
            if let Err(err) = image::save_buffer(
                &capture.path,
                &bytes,
                width as u32,
                height as u32,
                image::ColorType::Rgba8,
            ) {
                eprintln!("GoldWatcher: 截图保存失败：{err}");
            }
            self.capture = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !capture.requested && capture.started.elapsed() >= capture.delay {
            capture.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    fn alert_drawer(&mut self, ctx: &egui::Context) {
        if !self.show_alert_drawer || self.layout_mode != LayoutMode::TwoColumn {
            return;
        }
        let screen = ctx.screen_rect();
        let backdrop = egui::Area::new(Id::new("alerts-backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_black_alpha(100));
                response
            });
        if backdrop.inner.clicked() {
            self.show_alert_drawer = false;
        }
        let width = 370.0_f32.min(screen.width() - 24.0);
        let height = (screen.height() - 96.0).max(420.0);
        egui::Area::new(Id::new("alerts-drawer"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.right() - width - 16.0, 76.0))
            .show(ctx, |ui| {
                self.right_column(ui, width, height, true);
            });
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let logo = egui::load::SizedTexture::new(self.logo.id(), Vec2::splat(42.0));
            ui.add(egui::Image::from_texture(logo).corner_radius(11));
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("金瞳").size(21.0).color(theme::TEXT).strong());
                    ui.label(
                        RichText::new("GoldWatcher")
                            .size(13.0)
                            .color(theme::GOLD)
                            .strong(),
                    );
                });
                ui.label(theme::tiny("全球黄金市场 · 实时监控与阈值提醒"));
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(theme::ghost_button("设置")).clicked() {
                    self.show_settings = true;
                    self.settings_error = None;
                }
                if ui.add(theme::primary_button("↻ 刷新")).clicked() {
                    self.worker.refresh_now();
                    self.feed.last_attempt_instant = Some(Instant::now());
                }
                if self.layout_mode == LayoutMode::TwoColumn {
                    let label = format!("告警 {}", self.total_active_alerts());
                    if ui.add(theme::ghost_button(label)).clicked() {
                        self.show_alert_drawer = !self.show_alert_drawer;
                    }
                }
                let age = self
                    .quotes
                    .get(&MarketId::London)
                    .and_then(|quote| quote.quote_age_secs());
                let age_text = age.map(|secs| format!(" · {secs}s")).unwrap_or_default();
                let (status, color) = if !self.feed.online {
                    (format!("重试中{age_text}"), theme::RED)
                } else if self.feed.warning.is_some() {
                    (format!("降级{age_text}"), theme::GOLD)
                } else {
                    (format!("实时{age_text}"), theme::GREEN)
                };
                let pill = theme::status_pill(ui, status, color, true);
                let mut tooltip = format!(
                    "数据源：{}\n最近抓取：{}",
                    self.feed.source,
                    self.feed
                        .last_success
                        .map(|ts| ts.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "—".to_owned())
                );
                if let Some(age) = age {
                    tooltip.push_str(&format!("\n伦敦金报价延迟：{age} 秒"));
                }
                tooltip.push_str(&format!("\n抓取耗时：{} ms", self.feed.latency_ms));
                if let Some(warning) = &self.feed.warning {
                    tooltip.push_str(&format!("\n警告：{warning}"));
                }
                pill.on_hover_text(tooltip);
                ui.label(
                    RichText::new(now_beijing().format("%H:%M:%S").to_string())
                        .size(12.0)
                        .color(theme::MUTED)
                        .monospace(),
                );
            });
        });
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let body_h = (avail.y - 30.0).max(400.0);
        match self.layout_mode {
            LayoutMode::ThreeColumn => {
                let left_w = 292.0;
                let right_w = 340.0;
                let gap = 12.0;
                let center_w = (avail.x - left_w - right_w - gap * 2.0).max(520.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, body_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(left_w);
                            self.left_column(ui, body_h);
                        },
                    );
                    ui.add_space(gap);
                    ui.allocate_ui_with_layout(
                        Vec2::new(center_w, body_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(center_w);
                            self.center_column(ui, center_w, body_h);
                        },
                    );
                    ui.add_space(gap);
                    ui.allocate_ui_with_layout(
                        Vec2::new(right_w, body_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(right_w);
                            self.right_column(ui, right_w, body_h, false);
                        },
                    );
                });
            }
            LayoutMode::TwoColumn => {
                let left_w = 276.0;
                let gap = 12.0;
                let center_w = (avail.x - left_w - gap).max(560.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, body_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(left_w);
                            self.left_column(ui, body_h);
                        },
                    );
                    ui.add_space(gap);
                    ui.allocate_ui_with_layout(
                        Vec2::new(center_w, body_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(center_w);
                            self.center_column(ui, center_w, body_h);
                        },
                    );
                });
            }
            LayoutMode::Stacked => {
                egui::ScrollArea::vertical()
                    .id_salt("stacked-dashboard")
                    .auto_shrink([false, false])
                    .max_height(body_h)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let width = ui.available_width();
                        let chart_h = (width * 0.62).clamp(360.0, 600.0);
                        self.chart_section(ui, chart_h);
                        ui.add_space(12.0);
                        self.metrics_row(ui);
                        ui.add_space(12.0);
                        self.sessions_card(ui);
                        ui.add_space(12.0);
                        self.watchlist_card(ui, 356.0);
                        ui.add_space(12.0);
                        self.right_column(ui, width, 560.0, false);
                    });
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        theme::divider(ui);
        let remaining = self
            .feed
            .last_attempt_instant
            .map(|instant| {
                self.config
                    .poll_secs
                    .saturating_sub(instant.elapsed().as_secs())
            })
            .unwrap_or(self.config.poll_secs);
        let age = self
            .quotes
            .get(&MarketId::London)
            .and_then(|quote| quote.quote_age_secs());
        ui.horizontal(|ui| {
            let (online, color) = if self.feed.online {
                ("数据在线", theme::GREEN)
            } else {
                ("等待行情", theme::RED)
            };
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(5.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 2.5, color);
            ui.label(RichText::new(online).size(10.0).color(color));
            ui.add(egui::Label::new(theme::tiny(format!("· {}", self.feed.source))).truncate());
            if let Some(warning) = &self.feed.warning {
                ui.label(RichText::new("· 降级").size(10.0).color(theme::GOLD))
                    .on_hover_text(warning);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::tiny("行情仅供参考，不构成投资建议"));
                ui.label(theme::tiny(format!("下次刷新 {remaining}s")));
                let age_text = age
                    .map(|secs| format!("报价延迟 {secs}s"))
                    .unwrap_or_else(|| "报价延迟 —".to_owned());
                ui.label(theme::tiny(age_text));
                ui.label(theme::tiny(format!("抓取 {}ms", self.feed.latency_ms)));
            });
        });
    }

    fn left_column(&mut self, ui: &mut egui::Ui, height: f32) {
        let hero_h = 194.0;
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), hero_h),
            Layout::top_down(Align::Min),
            |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(hero_h - 32.0);
                    self.hero(ui);
                });
            },
        );
        ui.add_space(12.0);
        let list_h = (height - hero_h - 12.0).max(240.0);
        self.watchlist_card(ui, list_h);
    }

    fn watchlist_card(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), height),
            Layout::top_down(Align::Min),
            |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(height - 32.0);
                    self.watchlist(ui);
                });
            },
        );
    }

    fn chart_section(&mut self, ui: &mut egui::Ui, chart_h: f32) {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), chart_h),
            Layout::top_down(Align::Min),
            |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.chart_card(ui, chart_h);
                });
            },
        );
    }

    fn hero(&mut self, ui: &mut egui::Ui) {
        let market = self.config.selected_market;
        let quote = self.quotes.get(&market).cloned();
        let open = market.is_open(now_beijing());
        let points = self.history.points(market, ChartWindow::Hour1);

        ui.horizontal(|ui| {
            let color = if open { theme::GREEN } else { theme::FAINT };
            let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
            ui.painter().circle_filled(dot_rect.center(), 3.0, color);
            ui.label(theme::label(format!(
                "{} · {}",
                market.region(),
                if open { "交易中" } else { "休市" }
            )));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(market.code())
                        .size(10.5)
                        .color(theme::FAINT)
                        .monospace(),
                );
            });
        });
        ui.add_space(1.0);
        ui.label(RichText::new(market.name()).size(12.5).color(theme::MUTED));
        ui.add_space(2.0);

        let (price_text, change, change_pct) = match &quote {
            Some(q) if q.is_valid() => (market.fmt_price(q.price), q.change(), q.change_pct()),
            _ => ("—".to_owned(), None, None),
        };
        ui.horizontal(|ui| {
            ui.label(theme::big_money(price_text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(pct) = change_pct {
                    theme::change_chip(ui, pct, fmt_pct(pct));
                }
            });
        });
        let change_text = match (change, change_pct) {
            (Some(chg), Some(pct)) => {
                format!("{}  {}  ·  较昨收", market.fmt_signed(chg), fmt_pct(pct))
            }
            _ => "等待昨收数据".to_owned(),
        };
        ui.label(
            RichText::new(change_text)
                .size(11.0)
                .color(change.map(theme::direction_color).unwrap_or(theme::FAINT)),
        );
        ui.add_space(4.0);

        let spark_color = match (points.first(), points.last()) {
            (Some(first), Some(last)) if last.price >= first.price => theme::GREEN,
            (Some(_), Some(_)) => theme::RED,
            _ => theme::GOLD,
        };
        self.sparkline(ui, &points, 42.0, spark_color);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let meta = quote
                .as_ref()
                .map(|q| {
                    format!(
                        "开 {}   高 {}   低 {}",
                        market.fmt_price(q.open),
                        market.fmt_price(q.high),
                        market.fmt_price(q.low)
                    )
                })
                .unwrap_or_else(|| "等待行情接入".to_owned());
            ui.label(theme::tiny(meta));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(q) = &quote {
                    ui.label(theme::tiny(format!(
                        "{} · {}",
                        q.quote_time.clone().unwrap_or_else(|| "—".to_owned()),
                        q.freshness_text()
                    )))
                    .on_hover_text(format!(
                        "{}\n报价时间由数据源提供，延迟按本机时间计算",
                        q.source
                    ));
                }
            });
        });
    }

    fn sparkline(&self, ui: &mut egui::Ui, points: &[PricePoint], height: f32, color: Color32) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let painter = ui.painter_at(rect);
        if points.len() < 2 {
            painter.line_segment(
                [
                    Pos2::new(rect.left(), rect.center().y),
                    Pos2::new(rect.right(), rect.center().y),
                ],
                Stroke::new(1.0_f32, theme::BORDER),
            );
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "正在积累走势",
                FontId::proportional(9.0),
                theme::FAINT,
            );
            return;
        }
        let data = if points.len() > 180 {
            &points[points.len() - 180..]
        } else {
            points
        };
        let (min, max) = min_max(data.iter().map(|p| p.price));
        let plot = rect.shrink2(Vec2::new(1.0, 3.0));
        let x_for = |idx: usize| {
            plot.left() + plot.width() * idx as f32 / (data.len().saturating_sub(1)).max(1) as f32
        };
        let y_for = |price: f64| {
            let t = ((price - min) / (max - min).max(1e-9)) as f32;
            plot.bottom() - t * plot.height()
        };
        let mut mesh = egui::Mesh::default();
        for (i, p) in data.iter().enumerate() {
            let x = x_for(i);
            let top = Pos2::new(x, y_for(p.price));
            mesh.colored_vertex(top, theme::alpha(color, 55));
            mesh.colored_vertex(Pos2::new(x, plot.bottom()), theme::alpha(color, 0));
            if i > 0 {
                let base = (i as u32 - 1) * 2;
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base + 1, base + 3, base + 2);
            }
        }
        painter.add(Shape::mesh(mesh));
        let line: Vec<Pos2> = data
            .iter()
            .enumerate()
            .map(|(i, p)| Pos2::new(x_for(i), y_for(p.price)))
            .collect();
        painter.add(Shape::line(line.clone(), Stroke::new(1.7_f32, color)));
        if let Some(last) = line.last() {
            painter.circle_filled(*last, 2.8, color);
            painter.circle_filled(*last, 5.5, theme::alpha(color, 40));
        }
    }

    fn watchlist(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(theme::title("行情列表"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::tiny("5 个市场"));
            });
        });
        ui.add_space(2.0);
        let footer_h = 36.0;
        let status_h = 58.0;
        let rows_available = (ui.available_height() - footer_h - status_h).max(80.0);
        let row_h = ((rows_available - 6.0) / 5.0).clamp(46.0, 68.0);
        let max_rows_h = (row_h * 5.0 + 6.0).min(rows_available + 8.0);
        let mut selected = self.config.selected_market;
        egui::ScrollArea::vertical()
            .id_salt("watchlist")
            .max_height(max_rows_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for market in MarketId::ALL {
                    let quote = self.quotes.get(&market).cloned();
                    let is_selected = selected == market;
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let hovered = response.hovered();
                    let painter = ui.painter_at(rect);
                    let bg = if is_selected {
                        theme::SURFACE_2
                    } else if hovered {
                        theme::alpha(theme::SURFACE_2, 150)
                    } else {
                        Color32::TRANSPARENT
                    };
                    painter.rect_filled(rect.shrink2(Vec2::new(0.0, 2.0)), 10.0, bg);
                    if is_selected {
                        painter.rect_filled(
                            Rect::from_min_size(
                                Pos2::new(rect.left(), rect.top() + 11.0),
                                Vec2::new(2.5, row_h - 22.0),
                            ),
                            2.0,
                            theme::GOLD,
                        );
                    }
                    let open = market.is_open(now_beijing());
                    painter.circle_filled(
                        Pos2::new(rect.left() + 12.0, rect.center().y - 6.0),
                        2.6,
                        if open { theme::GREEN } else { theme::FAINT },
                    );
                    painter.text(
                        Pos2::new(rect.left() + 21.0, rect.center().y - 12.0),
                        Align2::LEFT_CENTER,
                        market.short_name(),
                        FontId::proportional(12.5),
                        if is_selected {
                            theme::TEXT
                        } else {
                            theme::MUTED
                        },
                    );
                    painter.text(
                        Pos2::new(rect.left() + 21.0, rect.center().y + 8.0),
                        Align2::LEFT_CENTER,
                        market.region(),
                        FontId::proportional(9.0),
                        theme::FAINT,
                    );

                    let (price, pct, quote_time) = match &quote {
                        Some(q) if q.is_valid() => (
                            market.fmt_price(q.price),
                            q.change_pct(),
                            q.quote_time.clone().unwrap_or_default(),
                        ),
                        _ => ("—".to_owned(), None, String::new()),
                    };
                    let pct_text = pct.map(fmt_pct).unwrap_or_else(|| "—".to_owned());
                    let pct_color = pct.map(theme::direction_color).unwrap_or(theme::FAINT);
                    let stale = quote
                        .as_ref()
                        .map(|q| q.age_secs() > (self.config.poll_secs as i64 * 3).max(45))
                        .unwrap_or(true);
                    painter.text(
                        Pos2::new(rect.right() - 8.0, rect.center().y - 12.0),
                        Align2::RIGHT_CENTER,
                        price,
                        FontId::proportional(15.0),
                        if stale { theme::MUTED } else { theme::TEXT },
                    );
                    painter.text(
                        Pos2::new(rect.right() - 8.0, rect.center().y + 8.0),
                        Align2::RIGHT_CENTER,
                        pct_text,
                        FontId::proportional(10.0),
                        pct_color,
                    );
                    if !quote_time.is_empty() {
                        painter.text(
                            Pos2::new(rect.left() + 21.0, rect.bottom() - 6.0),
                            Align2::LEFT_BOTTOM,
                            quote_time,
                            FontId::proportional(8.0),
                            theme::alpha(theme::FAINT, 170),
                        );
                    }
                    if response.clicked() && !is_selected {
                        selected = market;
                    }
                    response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    ui.add_space(1.0);
                }
            });

        if selected != self.config.selected_market {
            self.config.selected_market = selected;
            self.dirty = true;
            self.ensure_history(selected);
        }

        ui.add_space(4.0);
        theme::divider(ui);
        ui.horizontal(|ui| {
            let active = self.total_active_alerts();
            ui.label(theme::tiny(format!("监控中 {active} 条提醒")));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(theme::subtle_button("新建")).clicked() {
                    self.open_quick_alert();
                }
            });
        });
        ui.add_space(3.0);
        theme::divider(ui);
        ui.horizontal(|ui| {
            ui.label(theme::tiny("行情源"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&self.feed.source)
                            .size(9.0)
                            .color(theme::MUTED),
                    )
                    .truncate(),
                );
            });
        });
        ui.horizontal(|ui| {
            ui.label(theme::tiny("报价延迟 / 抓取耗时"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let age = self
                    .quotes
                    .get(&self.config.selected_market)
                    .and_then(|quote| quote.quote_age_secs())
                    .map(|secs| format!("{secs}s"))
                    .unwrap_or_else(|| "—".to_owned());
                ui.label(theme::tiny(format!("{} / {}ms", age, self.feed.latency_ms)));
            });
        });
    }

    fn center_column(&mut self, ui: &mut egui::Ui, width: f32, height: f32) {
        let metrics_h = 88.0;
        let sessions_h = 72.0;
        let gaps = 24.0;
        let available = (height - metrics_h - sessions_h - gaps).max(300.0);
        let width_cap = (width * 0.80).clamp(420.0, 650.0);
        let chart_h = available.min(width_cap).max(300.0);

        self.chart_section(ui, chart_h);
        ui.add_space(12.0);
        self.metrics_row(ui);
        ui.add_space(12.0);
        self.sessions_card(ui);
    }

    fn chart_card(&mut self, ui: &mut egui::Ui, chart_h: f32) {
        let market = self.config.selected_market;
        let quote = self.quotes.get(&market).cloned();
        let points = self.history.points(market, self.config.chart_window);
        let mut chosen = self.config.chart_window;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(market.name())
                            .size(17.0)
                            .color(theme::TEXT)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(market.code())
                            .size(10.0)
                            .color(theme::FAINT)
                            .monospace(),
                    );
                });
                ui.label(theme::tiny(format!(
                    "{} · {}",
                    market.region(),
                    if self.feed.online {
                        "实时行情"
                    } else {
                        "等待连接"
                    }
                )));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for window in ChartWindow::ALL.into_iter().rev() {
                    let selected = window == chosen;
                    let btn = egui::Button::new(
                        RichText::new(window.label()).size(10.5).color(if selected {
                            theme::GOLD
                        } else {
                            theme::MUTED
                        }),
                    )
                    .fill(if selected {
                        theme::alpha(theme::GOLD, 24)
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(
                        1.0_f32,
                        if selected {
                            theme::GOLD_DIM
                        } else {
                            theme::BORDER_SOFT
                        },
                    ))
                    .corner_radius(7);
                    if ui.add(btn).clicked() {
                        chosen = window;
                    }
                }
            });
        });
        if chosen != self.config.chart_window {
            self.config.chart_window = chosen;
            self.dirty = true;
        }
        ui.add_space(2.0);
        let mut chosen_style = self.config.chart_style;
        ui.horizontal(|ui| {
            let price = quote
                .as_ref()
                .map(|q| market.fmt_price(q.price))
                .unwrap_or_else(|| "—".to_owned());
            ui.label(RichText::new(price).size(26.0).color(theme::TEXT));
            if let Some(q) = &quote {
                if let Some(pct) = q.change_pct() {
                    theme::change_chip(ui, pct, fmt_pct(pct));
                }
            }
            ui.add_space(4.0);
            ui.label(theme::tiny("样式"));
            for style in ChartStyle::ALL {
                if style_button(ui, style, style == chosen_style) {
                    chosen_style = style;
                }
            }
            if ui.available_width() > 110.0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let source = quote
                        .as_ref()
                        .map(|q| q.source.clone())
                        .unwrap_or_else(|| self.feed.source.clone());
                    ui.add(egui::Label::new(theme::tiny(source)).truncate());
                });
            }
        });
        if chosen_style != self.config.chart_style {
            self.config.chart_style = chosen_style;
            self.dirty = true;
        }
        ui.add_space(4.0);

        let plot_h = (chart_h - 32.0 - 68.0 - 42.0).max(150.0);
        let prev_close = quote
            .as_ref()
            .map(|q| q.prev_close)
            .filter(|v| v.is_finite() && *v > 0.0);
        self.price_chart(ui, &points, prev_close, market, plot_h);
        ui.add_space(2.0);
        theme::divider(ui);

        ui.horizontal(|ui| {
            let stats = [
                ("开盘", quote.as_ref().map(|q| q.open)),
                ("最高", quote.as_ref().map(|q| q.high)),
                ("最低", quote.as_ref().map(|q| q.low)),
                ("昨收", prev_close),
            ];
            for (label, value) in stats {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(theme::tiny(label));
                    ui.label(
                        RichText::new(
                            value
                                .filter(|v| v.is_finite() && *v > 0.0)
                                .map(|v| market.fmt_price(v))
                                .unwrap_or_else(|| "—".to_owned()),
                        )
                        .size(11.5)
                        .color(theme::TEXT),
                    );
                });
                ui.add_space(10.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::tiny(format!(
                    "报价 {} · {}",
                    quote
                        .as_ref()
                        .and_then(|q| q.quote_time.clone())
                        .unwrap_or_else(|| "—".to_owned()),
                    quote
                        .as_ref()
                        .map(|q| q.freshness_text())
                        .unwrap_or_else(|| "—".to_owned())
                )));
            });
        });

        if let Some(notice) = &self.history_notice {
            if notice.contains(market.short_name()) {
                ui.label(
                    RichText::new(notice)
                        .size(9.0)
                        .color(theme::alpha(theme::GOLD, 190)),
                );
            }
        }
    }

    fn price_chart(
        &self,
        ui: &mut egui::Ui,
        points: &[PricePoint],
        prev_close: Option<f64>,
        market: MarketId,
        height: f32,
    ) {
        let style = self.config.chart_style;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let painter = ui.painter_at(rect);
        let plot = Rect::from_min_max(
            Pos2::new(rect.left() + 8.0, rect.top() + 25.0),
            Pos2::new(rect.right() - 64.0, rect.bottom() - 22.0),
        );
        if plot.width() < 100.0 || plot.height() < 50.0 {
            return;
        }
        painter.rect_filled(rect, 10.0, theme::alpha(theme::SURFACE_2, 45));

        if points.len() < 2 {
            draw_empty_chart(&painter, plot);
            return;
        }

        let last_price = points.last().map(|p| p.price).unwrap_or(0.0);
        let first_price = points.first().map(|p| p.price).unwrap_or(last_price);
        let reference = prev_close.unwrap_or(first_price);
        let trend_up = last_price >= reference;
        let trend_color = if trend_up { theme::GREEN } else { theme::RED };

        let (mut min, mut max) = min_max(points.iter().map(|p| p.price));
        if let Some(prev) = prev_close {
            let range = (max - min).max(0.0001);
            if style == ChartStyle::Baseline
                || (prev >= min - range * 0.3 && prev <= max + range * 0.3)
            {
                min = min.min(prev);
                max = max.max(prev);
            }
        }
        let pad = ((max - min) * 0.10).max(0.0001);
        min -= pad;
        max += pad;
        let first_ts = points.first().map(|p| p.ts).unwrap_or(0);
        let last_ts = points.last().map(|p| p.ts).unwrap_or(0);
        let span = (last_ts - first_ts).max(1) as f64;
        let x_for =
            |ts: i64| plot.left() + (((ts - first_ts) as f64 / span) * plot.width() as f64) as f32;
        let y_for = |price: f64| {
            let t = ((price - min) / (max - min).max(1e-9)) as f32;
            plot.bottom() - t.clamp(0.0, 1.0) * plot.height()
        };

        // Time grid + axis labels.
        let x_ticks = if plot.width() > 430.0 { 4 } else { 3 };
        for i in 0..=x_ticks {
            let t = i as f32 / x_ticks as f32;
            let ts = first_ts + ((last_ts - first_ts) as f32 * t) as i64;
            let x = x_for(ts);
            if i > 0 && i < x_ticks {
                painter.line_segment(
                    [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                    Stroke::new(1.0_f32, theme::alpha(theme::BORDER_SOFT, 95)),
                );
            }
            let align = if i == 0 {
                Align2::LEFT_TOP
            } else if i == x_ticks {
                Align2::RIGHT_TOP
            } else {
                Align2::CENTER_TOP
            };
            painter.text(
                Pos2::new(x, plot.bottom() + 6.0),
                align,
                beijing_clock(ts),
                FontId::monospace(9.0),
                theme::FAINT,
            );
        }

        // Horizontal grid + price axis.
        for i in 0..=4 {
            let t = i as f32 / 4.0;
            let y = plot.top() + t * plot.height();
            painter.line_segment(
                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(1.0_f32, theme::alpha(theme::BORDER_SOFT, 110)),
            );
            let value = max - (max - min) * t as f64;
            painter.text(
                Pos2::new(plot.right() + 6.0, y),
                Align2::LEFT_CENTER,
                axis_price(value, market, max - min),
                FontId::monospace(8.5),
                theme::FAINT,
            );
        }

        let last_x = x_for(last_ts);
        let last_y = y_for(last_price);

        // Chart body by style.
        match style {
            ChartStyle::Area => {
                let mesh = area_mesh(points, &x_for, &y_for, plot.bottom(), trend_color, 58);
                painter.add(Shape::mesh(mesh));
            }
            ChartStyle::Line => {}
            ChartStyle::Baseline => {
                let baseline = prev_close.unwrap_or(reference);
                let base_y = y_for(baseline).clamp(plot.top(), plot.bottom());
                let green = area_mesh(points, &x_for, &y_for, base_y, theme::GREEN, 55);
                let red = area_mesh(points, &x_for, &y_for, base_y, theme::RED, 55);
                let above = Rect::from_min_max(plot.min, Pos2::new(plot.right(), base_y));
                let below = Rect::from_min_max(Pos2::new(plot.left(), base_y), plot.max);
                if above.height() > 1.0 {
                    painter
                        .with_clip_rect(above.intersect(plot))
                        .add(Shape::mesh(green));
                }
                if below.height() > 1.0 {
                    painter
                        .with_clip_rect(below.intersect(plot))
                        .add(Shape::mesh(red));
                }
            }
            ChartStyle::Average => {
                let mesh = area_mesh(points, &x_for, &y_for, plot.bottom(), theme::GOLD, 26);
                painter.add(Shape::mesh(mesh));
            }
        }

        // Previous close reference.
        if let Some(prev) = prev_close {
            if prev >= min && prev <= max {
                let y = y_for(prev);
                dashed_line(
                    &painter,
                    Pos2::new(plot.left(), y),
                    Pos2::new(plot.right(), y),
                    theme::alpha(theme::MUTED, 110),
                );
                painter.text(
                    Pos2::new(plot.left() + 5.0, y - 5.0),
                    Align2::LEFT_BOTTOM,
                    format!("昨收 {}", market.fmt_price(prev)),
                    FontId::monospace(8.5),
                    theme::alpha(theme::MUTED, 210),
                );
            }
        }

        // Moving averages.
        if style == ChartStyle::Average {
            for (window, color) in [(300_i64, theme::GOLD), (1200_i64, theme::BLUE)] {
                let ma = moving_average(points, window);
                if ma.len() > 1 {
                    let line: Vec<Pos2> = ma
                        .iter()
                        .map(|p| Pos2::new(x_for(p.ts), y_for(p.price)))
                        .collect();
                    painter.add(Shape::line(line, Stroke::new(1.35_f32, color)));
                }
            }
        }

        // Main price line, with a soft glow.
        let price_color = if style == ChartStyle::Average {
            theme::TEXT
        } else {
            trend_color
        };
        let line: Vec<Pos2> = points
            .iter()
            .map(|p| Pos2::new(x_for(p.ts), y_for(p.price)))
            .collect();
        painter.add(Shape::line(
            line.clone(),
            Stroke::new(6.0_f32, theme::alpha(price_color, 26)),
        ));
        painter.add(Shape::line(
            line.clone(),
            Stroke::new(1.9_f32, theme::alpha(price_color, 235)),
        ));

        // High / low annotations.
        if let Some(high) = points.iter().max_by(|a, b| a.price.total_cmp(&b.price)) {
            if (high.ts - last_ts).abs() > 1 || (high.price - last_price).abs() > f64::EPSILON {
                let x = x_for(high.ts).clamp(plot.left() + 4.0, plot.right() - 4.0);
                painter.text(
                    Pos2::new(x, y_for(high.price) - 10.0),
                    Align2::CENTER_BOTTOM,
                    format!("高 {}", market.fmt_price(high.price)),
                    FontId::monospace(8.0),
                    theme::alpha(theme::MUTED, 180),
                );
            }
        }
        if let Some(low) = points.iter().min_by(|a, b| a.price.total_cmp(&b.price)) {
            if (low.ts - last_ts).abs() > 1 || (low.price - last_price).abs() > f64::EPSILON {
                let x = x_for(low.ts).clamp(plot.left() + 4.0, plot.right() - 4.0);
                painter.text(
                    Pos2::new(x, y_for(low.price) + 9.0),
                    Align2::CENTER_TOP,
                    format!("低 {}", market.fmt_price(low.price)),
                    FontId::monospace(8.0),
                    theme::alpha(theme::MUTED, 180),
                );
            }
        }

        // Latest price guide + right-side badge.
        dashed_line(
            &painter,
            Pos2::new(plot.left(), last_y),
            Pos2::new(plot.right(), last_y),
            theme::alpha(price_color, 110),
        );
        let pulse = 8.0 + (ui.input(|i| i.time) as f32 * 2.0).sin() * 1.5;
        painter.circle_filled(
            Pos2::new(last_x, last_y),
            pulse,
            theme::alpha(price_color, 24),
        );
        painter.circle_filled(Pos2::new(last_x, last_y), 3.2, theme::GOLD_BRIGHT);
        let badge = Rect::from_min_size(
            Pos2::new(plot.right() + 3.0, last_y - 8.0),
            Vec2::new(58.0, 16.0),
        );
        painter.rect_filled(badge, 4.0, price_color);
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            market.fmt_price(last_price),
            FontId::monospace(8.0),
            Color32::from_rgb(12, 17, 23),
        );

        // Style legend.
        let legend_y = rect.top() + 5.0;
        match style {
            ChartStyle::Average => {
                let ma5 = moving_average(points, 300);
                let ma20 = moving_average(points, 1200);
                let mut x = plot.left();
                for (label, color, value) in [
                    ("MA5", theme::GOLD, ma5.last().map(|p| p.price)),
                    ("MA20", theme::BLUE, ma20.last().map(|p| p.price)),
                ] {
                    painter.line_segment(
                        [
                            Pos2::new(x, legend_y + 5.0),
                            Pos2::new(x + 12.0, legend_y + 5.0),
                        ],
                        Stroke::new(1.5_f32, color),
                    );
                    painter.text(
                        Pos2::new(x + 16.0, legend_y),
                        Align2::LEFT_TOP,
                        format!(
                            "{} {}",
                            label,
                            value
                                .map(|v| fmt_num(v, 2))
                                .unwrap_or_else(|| "—".to_owned())
                        ),
                        FontId::monospace(8.0),
                        theme::alpha(color, 220),
                    );
                    x += 92.0;
                }
            }
            ChartStyle::Baseline => {
                let pct = prev_close
                    .filter(|p| *p > 0.0)
                    .map(|p| (last_price - p) / p * 100.0);
                painter.text(
                    Pos2::new(plot.left(), legend_y),
                    Align2::LEFT_TOP,
                    format!(
                        "现价 {}  {}",
                        market.fmt_price(last_price),
                        pct.map(fmt_pct).unwrap_or_else(|| "—".to_owned())
                    ),
                    FontId::monospace(8.5),
                    trend_color,
                );
            }
            ChartStyle::Area | ChartStyle::Line => {
                painter.text(
                    Pos2::new(plot.left(), legend_y),
                    Align2::LEFT_TOP,
                    style.description(),
                    FontId::proportional(8.5),
                    theme::alpha(theme::MUTED, 180),
                );
            }
        }

        // Hover crosshair + tooltip.
        if let Some(pointer) = response.hover_pos() {
            if plot.contains(pointer) {
                let target_ts = first_ts
                    + (((pointer.x - plot.left()) / plot.width().max(1.0)) as f64 * span) as i64;
                let nearest = points
                    .iter()
                    .min_by_key(|p| (p.ts - target_ts).abs())
                    .copied();
                if let Some(point) = nearest {
                    let px = x_for(point.ts);
                    let py = y_for(point.price);
                    dashed_line(
                        &painter,
                        Pos2::new(px, plot.top()),
                        Pos2::new(px, plot.bottom()),
                        theme::alpha(theme::MUTED, 90),
                    );
                    dashed_line(
                        &painter,
                        Pos2::new(plot.left(), py),
                        Pos2::new(plot.right(), py),
                        theme::alpha(theme::MUTED, 70),
                    );
                    painter.circle_filled(Pos2::new(px, py), 4.5, theme::BG);
                    painter.circle_filled(Pos2::new(px, py), 2.8, theme::GOLD_BRIGHT);

                    let pct = prev_close
                        .filter(|p| *p > 0.0)
                        .map(|p| (point.price - p) / p * 100.0);
                    let mut lines = vec![
                        format!(
                            "{}  {}",
                            beijing_clock(point.ts),
                            market.fmt_price(point.price)
                        ),
                        pct.map(fmt_pct).unwrap_or_else(|| "—".to_owned()),
                    ];
                    if style == ChartStyle::Average {
                        let ma5 = moving_average(points, 300);
                        let ma20 = moving_average(points, 1200);
                        if let (Some(a), Some(b)) = (ma5.last(), ma20.last()) {
                            lines.push(format!(
                                "MA5 {}   MA20 {}",
                                fmt_num(a.price, 2),
                                fmt_num(b.price, 2)
                            ));
                        }
                    }
                    let line_h = 13.0_f32;
                    let box_w = 160.0_f32;
                    let box_h = 10.0 + line_h * lines.len() as f32;
                    let mut pos = Pos2::new(pointer.x + 14.0, plot.top() + 8.0);
                    if pos.x + box_w > plot.right() {
                        pos.x = pointer.x - box_w - 14.0;
                    }
                    let box_rect = Rect::from_min_size(pos, Vec2::new(box_w, box_h));
                    painter.rect_filled(box_rect, 7.0, theme::alpha(theme::SURFACE_3, 248));
                    painter.rect_stroke(
                        box_rect,
                        CornerRadius::same(7),
                        Stroke::new(1.0_f32, theme::BORDER),
                        egui::StrokeKind::Inside,
                    );
                    for (index, line) in lines.iter().enumerate() {
                        painter.text(
                            box_rect.min + Vec2::new(8.0, 5.0 + index as f32 * line_h),
                            Align2::LEFT_TOP,
                            line,
                            FontId::monospace(8.5),
                            if index == 1 {
                                pct.map(theme::direction_color).unwrap_or(theme::MUTED)
                            } else {
                                theme::TEXT
                            },
                        );
                    }
                }
            }
        }
    }

    fn metrics_row(&mut self, ui: &mut egui::Ui) {
        let metrics = self.metrics.clone();
        let london = self.quotes.get(&MarketId::London);
        let basis_text = metrics
            .comex_basis
            .map(|v| {
                format!(
                    "{}{}",
                    if v >= 0.0 { "+" } else { "-" },
                    theme_money(v.abs())
                )
            })
            .unwrap_or_else(|| "—".to_owned());
        let basis_sub = metrics
            .comex_basis_pct
            .map(|v| format!("{} · 纽约−伦敦", fmt_pct(v)))
            .unwrap_or_else(|| "等待纽约与伦敦行情".to_owned());
        let basis_color = metrics
            .comex_basis
            .map(theme::direction_color)
            .unwrap_or(theme::FAINT);

        let premium_text = metrics
            .sge_premium_pct
            .map(fmt_pct)
            .unwrap_or_else(|| "—".to_owned());
        let premium_sub = metrics
            .sge_premium_cny
            .map(|v| format!("折算价差 {} 元/克", fmt_num(v, 2)))
            .unwrap_or_else(|| "等待汇率与上海金".to_owned());
        let premium_color = metrics
            .sge_premium_pct
            .map(theme::direction_color)
            .unwrap_or(theme::FAINT);

        let fx_text = metrics
            .usd_cny
            .map(|v| fmt_num(v, 4))
            .unwrap_or_else(|| "—".to_owned());
        let fx_sub = self
            .fx
            .as_ref()
            .map(|f| format!("{} · 每日更新", f.source))
            .unwrap_or_else(|| "等待汇率数据".to_owned());

        let amplitude = london.and_then(|q| q.range_pct());
        let amp_text = amplitude
            .map(|v| format!("{:.2}%", v))
            .unwrap_or_else(|| "—".to_owned());
        let amp_sub = london
            .map(|q| {
                format!(
                    "高 {} / 低 {}",
                    MarketId::London.fmt_price(q.high),
                    MarketId::London.fmt_price(q.low)
                )
            })
            .unwrap_or_else(|| "等待伦敦金行情".to_owned());

        let cards = [
            ("COMEX 基差", basis_text, basis_sub, basis_color),
            ("上海金溢价", premium_text, premium_sub, premium_color),
            ("美元 / 人民币", fx_text, fx_sub, theme::GOLD),
            ("伦敦金今日振幅", amp_text, amp_sub, theme::BLUE),
        ];
        let columns = if ui.available_width() >= 560.0 { 4 } else { 2 };
        let gap = 10.0;
        let width = (ui.available_width() - gap * (columns - 1) as f32) / columns as f32;
        let rows: Vec<_> = cards.chunks(columns).collect();
        for (row_index, row) in rows.iter().enumerate() {
            ui.horizontal(|ui| {
                for (i, (label, value, sub, color)) in row.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(gap);
                    }
                    metric_card(ui, width, label, value, sub, *color);
                }
            });
            if row_index + 1 < rows.len() {
                ui.add_space(8.0);
            }
        }
    }

    fn sessions_card(&mut self, ui: &mut egui::Ui) {
        let now = now_beijing();
        let sessions = [
            ("伦敦", "15:00–24:00", MarketId::London.is_open(now)),
            ("纽约", "06:00–05:00", MarketId::NewYork.is_open(now)),
            (
                "上海",
                "09:00–15:30 / 20:00–02:30",
                MarketId::Shanghai.is_open(now),
            ),
            (
                "东京",
                "08:00–14:15 / 16:00–05:00",
                session_range_open(now, 8 * 60, 14 * 60 + 15)
                    || session_range_open(now, 16 * 60, 5 * 60),
            ),
            (
                "印度",
                "11:30–02:00",
                session_range_open(now, 11 * 60 + 30, 2 * 60),
            ),
            (
                "迪拜",
                "11:00–03:00",
                session_range_open(now, 11 * 60, 3 * 60),
            ),
        ];

        theme::card_tight().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::tiny("全球时段 · 北京时间"));
            ui.add_space(2.0);
            ui.columns(6, |columns| {
                for (index, (name, range, open)) in sessions.into_iter().enumerate() {
                    let ui = &mut columns[index];
                    let color = if open { theme::GREEN } else { theme::FAINT };
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.horizontal(|ui| {
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(5.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 2.5, color);
                            ui.label(RichText::new(name).size(9.5).color(if open {
                                theme::TEXT
                            } else {
                                theme::MUTED
                            }));
                        });
                        ui.label(
                            RichText::new(if open { "开市" } else { "休市" })
                                .size(8.5)
                                .color(color),
                        );
                    })
                    .response
                    .on_hover_text(format!(
                        "{name}交易时段（北京时间）：{range}\n当前{}",
                        if open { "开市" } else { "休市" }
                    ));
                }
            });
        });
    }

    fn right_column(&mut self, ui: &mut egui::Ui, _width: f32, height: f32, drawer: bool) {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), height),
            Layout::top_down(Align::Min),
            |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(height - 32.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(theme::title("告警中心"));
                            ui.label(theme::tiny("价格阈值、开收盘、跨市场价差"));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let active = self.total_active_alerts();
                            theme::status_pill(
                                ui,
                                format!("{active} 监控中"),
                                if active > 0 {
                                    theme::GOLD
                                } else {
                                    theme::FAINT
                                },
                                true,
                            );
                            if drawer && ui.add(theme::ghost_button("×")).clicked() {
                                self.show_alert_drawer = false;
                            }
                        });
                    });
                    ui.add_space(2.0);
                    let new_label = if self.right_tab == RightTab::Sessions {
                        "＋ 新建开收盘提醒"
                    } else {
                        "＋ 新建价格告警"
                    };
                    if ui
                        .add_sized(
                            Vec2::new(ui.available_width(), 34.0),
                            theme::primary_button(new_label),
                        )
                        .clicked()
                    {
                        if self.right_tab == RightTab::Sessions {
                            self.open_session_editor();
                        } else {
                            self.open_quick_alert();
                        }
                    }
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if tab_button(ui, "价格告警", self.right_tab == RightTab::Alerts) {
                            self.right_tab = RightTab::Alerts;
                        }
                        if tab_button(ui, "开收盘", self.right_tab == RightTab::Sessions) {
                            self.right_tab = RightTab::Sessions;
                        }
                        if tab_button(ui, "触发记录", self.right_tab == RightTab::Events) {
                            self.right_tab = RightTab::Events;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let count = match self.right_tab {
                                RightTab::Alerts => self.config.alerts.len(),
                                RightTab::Sessions => self.config.session_alerts.len(),
                                RightTab::Events => self.config.events.len(),
                            };
                            ui.label(theme::tiny(format!("{count} 条")));
                        });
                    });
                    theme::divider(ui);

                    let footer_h = 24.0;
                    let scroll_h = (ui.available_height() - footer_h).max(120.0);
                    match self.right_tab {
                        RightTab::Alerts => self.alerts_list(ui, scroll_h),
                        RightTab::Sessions => self.sessions_list(ui, scroll_h),
                        RightTab::Events => self.events_list(ui, scroll_h),
                    }
                    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            let color = if self.config.notifications {
                                theme::GREEN
                            } else {
                                theme::FAINT
                            };
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(5.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 2.5, color);
                            ui.label(theme::tiny(if self.config.notifications {
                                "桌面通知已开启 · 触发后即时提醒"
                            } else {
                                "桌面通知已关闭 · 仅在应用内提示"
                            }));
                        });
                    });
                });
            },
        );
    }

    fn alerts_list(&mut self, ui: &mut egui::Ui, height: f32) {
        if self.config.alerts.is_empty() {
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), height),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.add_space((height * 0.12).clamp(8.0, 40.0));
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    ui.painter().circle_stroke(
                        rect.center(),
                        20.0,
                        Stroke::new(1.0_f32, theme::alpha(theme::GOLD, 80)),
                    );
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "!",
                        FontId::proportional(20.0),
                        theme::GOLD,
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new("还没有告警").size(13.0).color(theme::TEXT));
                    ui.label(theme::tiny("设置阈值，让 GoldWatcher 替你盯盘"));
                    ui.add_space(10.0);
                    self.quick_templates(ui);
                },
            );
            return;
        }

        let alerts = self.config.alerts.clone();
        let quotes = self.quotes.clone();
        let metrics = self.metrics.clone();
        let confirm = self.delete_confirm.clone();
        let mut actions = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("alerts-list")
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for alert in &alerts {
                    let current = alert.current_text(&quotes, &metrics);
                    if let Some(action) = alert_card(
                        ui,
                        alert,
                        &current,
                        confirm.as_deref() == Some(alert.id.as_str()),
                    ) {
                        actions.push(action);
                    }
                    ui.add_space(7.0);
                }
            });
        for action in actions {
            self.apply_alert_action(action);
        }
    }

    fn sessions_list(&mut self, ui: &mut egui::Ui, height: f32) {
        if self.config.session_alerts.is_empty() {
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), height),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.add_space((height * 0.08).clamp(6.0, 28.0));
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    ui.painter().circle_stroke(
                        rect.center(),
                        20.0,
                        Stroke::new(1.0_f32, theme::alpha(theme::BLUE, 80)),
                    );
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "◷",
                        FontId::proportional(20.0),
                        theme::BLUE,
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("还没有开收盘提醒")
                            .size(13.0)
                            .color(theme::TEXT),
                    );
                    ui.label(theme::tiny("市场开市、收市时提醒你关注行情"));
                    ui.add_space(8.0);
                    self.session_templates(ui);
                },
            );
            return;
        }

        let alerts = self.config.session_alerts.clone();
        let confirm = self.delete_confirm.clone();
        let now = now_beijing();
        let mut actions = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("sessions-list")
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for alert in &alerts {
                    if let Some(action) = session_alert_card(
                        ui,
                        alert,
                        confirm.as_deref() == Some(alert.id.as_str()),
                        now,
                    ) {
                        actions.push(action);
                    }
                    ui.add_space(7.0);
                }
            });
        for action in actions {
            self.apply_session_action(action);
        }
    }

    fn session_templates(&mut self, ui: &mut egui::Ui) {
        let mut add: Option<(SessionId, bool, bool)> = None;
        if template_button(ui, "纽约 COMEX 开 / 收盘", "06:00 开盘 · 次日 05:00 收盘") {
            add = Some((SessionId::NewYork, true, true));
        }
        if template_button(ui, "上海日盘开盘", "09:00 开盘提醒 · 北京时间") {
            add = Some((SessionId::ShanghaiDay, true, false));
        }
        if template_button(ui, "伦敦盘收盘", "24:00 收盘提醒 · 北京时间") {
            add = Some((SessionId::London, false, true));
        }
        if template_button(ui, "上海夜盘开 / 收盘", "20:00 开盘 · 次日 02:30 收盘") {
            add = Some((SessionId::ShanghaiNight, true, true));
        }
        if let Some((session, watch_open, watch_close)) = add {
            let mut alert = SessionAlert::new(session, watch_open, watch_close);
            alert.notify = self.config.notifications;
            alert.flash = self.config.flash;
            self.config.session_alerts.insert(0, alert);
            self.dirty = true;
        }
    }

    fn quick_templates(&mut self, ui: &mut egui::Ui) {
        let selected = self.config.selected_market;
        let price = self
            .quotes
            .get(&selected)
            .filter(|q| q.is_valid())
            .map(|q| q.price);
        let mut add: Option<(AlertScope, AlertMetric, Direction, f64, &'static str)> = None;

        if let Some(price) = price {
            let threshold = round_to(price * 1.01, 2);
            if template_button(
                ui,
                "现价上方 1%",
                &format!(
                    "{} 突破 {}",
                    selected.short_name(),
                    selected.fmt_price(threshold)
                ),
            ) {
                add = Some((
                    AlertScope::Market { market: selected },
                    AlertMetric::Price,
                    Direction::Above,
                    threshold,
                    "现价上方 1%",
                ));
            }
        }
        if template_button(
            ui,
            "日内涨幅 1%",
            &format!("{} 日涨跌幅 ≥ +1.00%", selected.short_name()),
        ) {
            add = Some((
                AlertScope::Market { market: selected },
                AlertMetric::DayChangePct,
                Direction::Above,
                1.0,
                "日内涨幅超过 1%",
            ));
        }
        if template_button(ui, "上海金溢价 1%", "内盘较国际金价折算溢价 ≥ +1.00%")
        {
            add = Some((
                AlertScope::SgePremium,
                AlertMetric::Price,
                Direction::Above,
                1.0,
                "上海金溢价超过 1%",
            ));
        }

        if let Some((scope, metric, direction, threshold, name)) = add {
            self.quick_add(scope, metric, direction, threshold, name);
        }
    }

    fn events_list(&mut self, ui: &mut egui::Ui, height: f32) {
        let mut changed = false;
        for event in &mut self.config.events {
            if !event.read {
                event.read = true;
                changed = true;
            }
        }
        if changed {
            self.dirty = true;
        }
        ui.horizontal(|ui| {
            ui.label(theme::tiny("最近 100 条触发记录"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.clear_events_confirm {
                    if ui.add(theme::ghost_button("确认清空")).clicked() {
                        self.config.events.clear();
                        self.clear_events_confirm = false;
                        self.dirty = true;
                    }
                    if ui.add(theme::subtle_button("取消")).clicked() {
                        self.clear_events_confirm = false;
                    }
                } else if !self.config.events.is_empty()
                    && ui.add(theme::subtle_button("清空")).clicked()
                {
                    self.clear_events_confirm = true;
                }
            });
        });
        ui.add_space(4.0);
        if self.config.events.is_empty() {
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), height - 30.0),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.add_space((height * 0.2).clamp(10.0, 60.0));
                    ui.label(RichText::new("暂无触发记录").size(13.0).color(theme::MUTED));
                    ui.label(theme::tiny("条件满足后，提醒会出现在这里"));
                },
            );
            return;
        }
        let events = self.config.events.clone();
        egui::ScrollArea::vertical()
            .id_salt("events-list")
            .max_height(height - 30.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for event in &events {
                    Frame::new()
                        .fill(theme::SURFACE_2)
                        .stroke(Stroke::new(1.0_f32, theme::BORDER_SOFT))
                        .corner_radius(10)
                        .inner_margin(Margin::symmetric(11, 9))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(&event.title)
                                        .size(11.5)
                                        .color(theme::GOLD)
                                        .strong(),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(theme::tiny(
                                        event.triggered_at.format("%m-%d %H:%M:%S").to_string(),
                                    ));
                                });
                            });
                            ui.label(RichText::new(&event.message).size(10.5).color(theme::MUTED));
                        });
                    ui.add_space(6.0);
                }
            });
    }
}

impl AlertEditor {
    fn metric_label_for_editor(&self) -> &'static str {
        match self.scope {
            AlertScope::Market { .. } => self.metric.label(),
            AlertScope::ComexBasis => "基差",
            AlertScope::SgePremium => "溢价",
            AlertScope::UsdCny => "汇率",
        }
    }
}

impl GoldWatcherApp {
    fn alert_editor_modal(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let quotes = self.quotes.clone();
        let metrics = self.metrics.clone();
        let is_edit = editor.edit_id.is_some();
        let mut close = false;
        let mut save = false;

        let modal = egui::Modal::new(Id::new("alert-editor-modal"))
            .frame(theme::card().inner_margin(Margin::same(22)))
            .show(ctx, |ui| {
                ui.set_width(468.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(
                            RichText::new(if is_edit {
                                "编辑告警"
                            } else {
                                "新建告警"
                            })
                            .size(19.0)
                            .color(theme::TEXT)
                            .strong(),
                        );
                        ui.label(theme::tiny("条件满足时提醒一次，可设置为持续重复提醒"));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(theme::gold_label("ALERT RULE"));
                    });
                });
                ui.add_space(10.0);
                theme::divider(ui);
                ui.add_space(4.0);

                ui.label(theme::label("告警名称（可选）"));
                ui.add(
                    egui::TextEdit::singleline(&mut editor.name)
                        .hint_text("例如：伦敦金突破 4,200")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);
                ui.label(theme::label("监控对象"));
                let mut scope_changed = false;
                let current_scope = editor.scope;
                egui::ComboBox::from_id_salt("alert-scope")
                    .width(240.0)
                    .selected_text(current_scope.label())
                    .show_ui(ui, |ui| {
                        for market in MarketId::ALL {
                            let scope = AlertScope::Market { market };
                            if ui
                                .selectable_value(&mut editor.scope, scope, market.name())
                                .changed()
                            {
                                scope_changed = true;
                            }
                        }
                        if ui
                            .selectable_value(
                                &mut editor.scope,
                                AlertScope::ComexBasis,
                                "COMEX 基差",
                            )
                            .changed()
                        {
                            scope_changed = true;
                        }
                        if ui
                            .selectable_value(
                                &mut editor.scope,
                                AlertScope::SgePremium,
                                "上海金溢价",
                            )
                            .changed()
                        {
                            scope_changed = true;
                        }
                        if ui
                            .selectable_value(
                                &mut editor.scope,
                                AlertScope::UsdCny,
                                "美元 / 人民币",
                            )
                            .changed()
                        {
                            scope_changed = true;
                        }
                    });
                if scope_changed {
                    editor.metric = AlertMetric::Price;
                    editor.threshold = default_threshold(editor.scope, &quotes, &metrics);
                }

                ui.add_space(4.0);
                egui::Grid::new("alert-editor-grid")
                    .num_columns(2)
                    .spacing(Vec2::new(18.0, 9.0))
                    .show(ui, |ui| {
                        if editor.scope.is_market() {
                            ui.label(theme::label("指标"));
                            let metric_before = editor.metric;
                            egui::ComboBox::from_id_salt("alert-metric")
                                .width(150.0)
                                .selected_text(editor.metric.label())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut editor.metric,
                                        AlertMetric::Price,
                                        "价格",
                                    );
                                    ui.selectable_value(
                                        &mut editor.metric,
                                        AlertMetric::DayChangePct,
                                        "日涨跌幅",
                                    );
                                });
                            if editor.metric != metric_before {
                                editor.threshold = match editor.metric {
                                    AlertMetric::DayChangePct => 1.0,
                                    AlertMetric::Price => {
                                        default_threshold(editor.scope, &quotes, &metrics)
                                    }
                                };
                            }
                            ui.end_row();
                        }
                        ui.label(theme::label("条件"));
                        egui::ComboBox::from_id_salt("alert-direction")
                            .width(150.0)
                            .selected_text(editor.direction.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut editor.direction,
                                    Direction::Above,
                                    "高于 / 上破",
                                );
                                ui.selectable_value(
                                    &mut editor.direction,
                                    Direction::Below,
                                    "低于 / 跌破",
                                );
                            });
                        ui.end_row();

                        ui.label(theme::label("阈值"));
                        ui.horizontal(|ui| {
                            threshold_widget(ui, &mut editor);
                            ui.label(theme::tiny(threshold_unit(editor.scope, editor.metric)));
                        });
                        ui.end_row();
                    });

                let current = match (editor.scope, editor.metric) {
                    (AlertScope::Market { market }, AlertMetric::Price) => quotes
                        .get(&market)
                        .map(|q| market.fmt_price(q.price))
                        .unwrap_or_else(|| "—".to_owned()),
                    (AlertScope::Market { market }, AlertMetric::DayChangePct) => quotes
                        .get(&market)
                        .and_then(|q| q.change_pct())
                        .map(fmt_pct)
                        .unwrap_or_else(|| "—".to_owned()),
                    (AlertScope::ComexBasis, _) => metrics
                        .comex_basis
                        .map(|v| {
                            format!(
                                "{}{}",
                                if v >= 0.0 { "+" } else { "-" },
                                theme_money(v.abs())
                            )
                        })
                        .unwrap_or_else(|| "—".to_owned()),
                    (AlertScope::SgePremium, _) => metrics
                        .sge_premium_pct
                        .map(fmt_pct)
                        .unwrap_or_else(|| "—".to_owned()),
                    (AlertScope::UsdCny, _) => metrics
                        .usd_cny
                        .map(|v| fmt_num(v, 4))
                        .unwrap_or_else(|| "—".to_owned()),
                };
                ui.label(theme::tiny(format!(
                    "当前值 {current} · 数据每次刷新后重新判断"
                )));

                ui.add_space(6.0);
                theme::divider(ui);
                ui.add_space(4.0);
                ui.checkbox(
                    &mut editor.repeat,
                    RichText::new("持续满足时重复提醒")
                        .size(12.0)
                        .color(theme::TEXT),
                );
                if editor.repeat {
                    ui.horizontal(|ui| {
                        ui.label(theme::tiny("重复冷却"));
                        ui.add(
                            egui::DragValue::new(&mut editor.cooldown_secs)
                                .range(30..=86_400)
                                .speed(10.0)
                                .suffix(" 秒"),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut editor.notify,
                        RichText::new("桌面通知").size(12.0).color(theme::TEXT),
                    );
                    ui.checkbox(
                        &mut editor.flash,
                        RichText::new("窗口闪烁").size(12.0).color(theme::TEXT),
                    );
                    ui.checkbox(
                        &mut editor.enabled,
                        RichText::new("立即启用").size(12.0).color(theme::TEXT),
                    );
                });

                if let Some(error) = &editor.error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(error).size(11.0).color(theme::RED));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(Vec2::new(96.0, 32.0), theme::primary_button("保存告警"))
                        .clicked()
                    {
                        if validate_editor(&editor) {
                            save = true;
                            close = true;
                        } else {
                            editor.error = Some(editor_validation_error(&editor));
                        }
                    }
                    if ui
                        .add_sized(Vec2::new(76.0, 32.0), theme::ghost_button("取消"))
                        .clicked()
                    {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(theme::tiny("提醒仅供信息参考"));
                    });
                });
            });

        if modal.should_close() {
            close = true;
        }
        if save {
            self.commit_editor(editor);
        } else if !close {
            self.editor = Some(editor);
        }
    }

    fn session_editor_modal(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.session_editor.take() else {
            return;
        };
        let is_edit = editor.edit_id.is_some();
        let mut close = false;
        let mut save = false;
        let now = now_beijing();
        let modal = egui::Modal::new(Id::new("session-editor-modal"))
            .frame(theme::card().inner_margin(Margin::same(22)))
            .show(ctx, |ui| {
                ui.set_width(448.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(
                            RichText::new(if is_edit {
                                "编辑开收盘提醒"
                            } else {
                                "新建开收盘提醒"
                            })
                            .size(19.0)
                            .color(theme::TEXT)
                            .strong(),
                        );
                        ui.label(theme::tiny("在全球主要黄金交易时段开始或结束时提醒"));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(theme::gold_label("SESSION"));
                    });
                });
                ui.add_space(10.0);
                theme::divider(ui);
                ui.add_space(4.0);

                ui.label(theme::label("提醒名称（可选）"));
                ui.add(
                    egui::TextEdit::singleline(&mut editor.name)
                        .hint_text("例如：纽约 COMEX 开收盘")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);
                ui.label(theme::label("交易时段"));
                egui::ComboBox::from_id_salt("session-market")
                    .width(240.0)
                    .selected_text(editor.session.name())
                    .show_ui(ui, |ui| {
                        for session in SessionId::ALL {
                            ui.selectable_value(
                                &mut editor.session,
                                session,
                                format!("{}  ·  {}", session.name(), session.schedule_text()),
                            );
                        }
                    });
                let open = editor.session.is_open(now);
                ui.horizontal(|ui| {
                    let color = if open { theme::GREEN } else { theme::FAINT };
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(5.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 2.5, color);
                    ui.label(theme::tiny(format!(
                        "{} · 当前{} · {}（北京时间）",
                        editor.session.description(),
                        if open { "开市" } else { "休市" },
                        editor.session.schedule_text()
                    )));
                });

                ui.add_space(4.0);
                ui.label(theme::label("触发时机"));
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut editor.watch_open,
                        RichText::new("开盘时提醒").size(12.0).color(theme::TEXT),
                    );
                    ui.checkbox(
                        &mut editor.watch_close,
                        RichText::new("收盘时提醒").size(12.0).color(theme::TEXT),
                    );
                });
                if !editor.watch_open && !editor.watch_close {
                    ui.label(
                        RichText::new("请至少选择开盘或收盘中的一个")
                            .size(10.5)
                            .color(theme::RED),
                    );
                }
                if let Some(error) = &editor.error {
                    ui.label(RichText::new(error).size(10.5).color(theme::RED));
                }

                ui.add_space(6.0);
                theme::divider(ui);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut editor.notify,
                        RichText::new("桌面通知").size(12.0).color(theme::TEXT),
                    );
                    ui.checkbox(
                        &mut editor.flash,
                        RichText::new("窗口闪烁").size(12.0).color(theme::TEXT),
                    );
                    ui.checkbox(
                        &mut editor.enabled,
                        RichText::new("立即启用").size(12.0).color(theme::TEXT),
                    );
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(Vec2::new(104.0, 32.0), theme::primary_button("保存提醒"))
                        .clicked()
                    {
                        if editor.watch_open || editor.watch_close {
                            save = true;
                            close = true;
                        } else {
                            editor.error = Some("请至少选择开盘或收盘中的一个".to_owned());
                        }
                    }
                    if ui
                        .add_sized(Vec2::new(76.0, 32.0), theme::ghost_button("取消"))
                        .clicked()
                    {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(theme::tiny("时段边界按北京时间近似计算"));
                    });
                });
            });
        if modal.should_close() {
            close = true;
        }
        if save {
            self.commit_session_editor(editor);
        } else if !close {
            self.session_editor = Some(editor);
        }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut close = false;
        let mut save_now = false;
        let modal = egui::Modal::new(Id::new("settings-modal"))
            .frame(theme::card().inner_margin(Margin::same(22)))
            .show(ctx, |ui| {
                ui.set_width(476.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("设置").size(19.0).color(theme::TEXT).strong());
                        ui.label(theme::tiny("行情、通知与桌面集成"));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(theme::gold_label("PREFERENCES"));
                    });
                });
                ui.add_space(8.0);
                theme::divider(ui);
                ui.add_space(4.0);

                ui.label(theme::label("行情刷新"));
                ui.horizontal(|ui| {
                    let mut poll = self.config.poll_secs;
                    egui::ComboBox::from_id_salt("poll-secs")
                        .width(118.0)
                        .selected_text(if poll == 15 {
                            "15 秒（默认）".to_owned()
                        } else {
                            format!("{poll} 秒")
                        })
                        .show_ui(ui, |ui| {
                            for secs in POLL_OPTIONS {
                                let label = if secs == 15 {
                                    "15 秒（默认）".to_owned()
                                } else {
                                    format!("{secs} 秒")
                                };
                                ui.selectable_value(&mut poll, secs, label);
                            }
                        });
                    if poll != self.config.poll_secs {
                        self.config.poll_secs = normalize_poll_secs(poll);
                        self.worker.set_poll_secs(self.config.poll_secs);
                        self.dirty = true;
                    }
                    ui.label(theme::tiny("行情时效优先，15 秒档最灵敏"));
                });

                ui.add_space(4.0);
                ui.label(theme::label("提醒方式"));
                if ui
                    .checkbox(
                        &mut self.config.notifications,
                        RichText::new("桌面系统通知").size(12.0).color(theme::TEXT),
                    )
                    .changed()
                {
                    self.dirty = true;
                }
                if ui
                    .checkbox(
                        &mut self.config.flash,
                        RichText::new("触发时请求窗口注意")
                            .size(12.0)
                            .color(theme::TEXT),
                    )
                    .changed()
                {
                    self.dirty = true;
                }

                ui.add_space(4.0);
                ui.label(theme::label("系统集成"));
                if storage::autostart_supported() {
                    let mut autostart = self.config.auto_start;
                    if ui
                        .checkbox(
                            &mut autostart,
                            RichText::new("开机自动启动 GoldWatcher")
                                .size(12.0)
                                .color(theme::TEXT),
                        )
                        .changed()
                    {
                        match storage::set_autostart(autostart) {
                            Ok(()) => {
                                self.config.auto_start = autostart;
                                self.dirty = true;
                                self.settings_error = None;
                            }
                            Err(err) => self.settings_error = Some(err),
                        }
                    }
                } else {
                    ui.label(theme::tiny("当前平台请在系统登录项中配置开机启动"));
                }
                ui.horizontal(|ui| {
                    let color = if self.tray_available {
                        theme::GREEN
                    } else {
                        theme::FAINT
                    };
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(5.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 2.5, color);
                    ui.label(theme::tiny(if self.tray_available {
                        "系统托盘已启用：右键托盘图标可快速刷新或退出"
                    } else {
                        "系统托盘不可用（未检测到 StatusNotifier 服务）"
                    }));
                });
                if let Some(err) = &self.settings_error {
                    ui.label(RichText::new(err).size(10.5).color(theme::RED));
                }

                ui.add_space(4.0);
                ui.label(theme::label("数据与存储"));
                ui.horizontal(|ui| {
                    if ui.add(theme::subtle_button("打开数据目录")).clicked() {
                        storage::open_path(&self.paths.config_dir);
                    }
                    ui.label(theme::tiny(format!(
                        "配置：{}",
                        self.paths.config_file.display()
                    )));
                });
                ui.label(theme::tiny(format!(
                    "本次行情耗时 {} ms · 最近成功 {}",
                    self.feed.latency_ms,
                    self.feed
                        .last_success
                        .map(|ts| ts.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "—".to_owned())
                )));
                ui.label(theme::tiny(
                    "时效优先：Gold-API 秒级现货 + 新浪多市场并发抓取",
                ));
                ui.label(theme::tiny(
                    "数据源：新浪财经 · 上海黄金交易所 · ExchangeRate-API · Gold-API",
                ));
                ui.label(theme::tiny("行情可能延迟，仅供信息参考，不构成投资建议。"));
                if let Some(notice) = &self.history_notice {
                    ui.label(
                        RichText::new(notice)
                            .size(10.0)
                            .color(theme::alpha(theme::GOLD, 200)),
                    );
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(Vec2::new(96.0, 32.0), theme::primary_button("保存并关闭"))
                        .clicked()
                    {
                        close = true;
                        save_now = true;
                    }
                    if ui
                        .add_sized(Vec2::new(66.0, 32.0), theme::ghost_button("关闭"))
                        .clicked()
                    {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(theme::tiny(format!(
                            "GoldWatcher v{}",
                            env!("CARGO_PKG_VERSION")
                        )));
                    });
                });
            });
        if modal.should_close() {
            close = true;
        }
        if close {
            self.show_settings = false;
            if save_now {
                self.save_now();
            }
        }
    }

    fn show_toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|toast| toast.expires > now);
        let toasts = self.toasts.clone();
        for (index, toast) in toasts.iter().enumerate() {
            let response = egui::Area::new(Id::new(("goldwatcher-toast", index)))
                .order(egui::Order::Foreground)
                .anchor(
                    Align2::RIGHT_TOP,
                    Vec2::new(-24.0, 76.0 + index as f32 * 104.0),
                )
                .show(ctx, |ui| {
                    Frame::new()
                        .fill(theme::alpha(theme::SURFACE_3, 248))
                        .stroke(Stroke::new(1.0_f32, theme::alpha(theme::GOLD, 140)))
                        .corner_radius(12)
                        .inner_margin(Margin::symmetric(13, 10))
                        .show(ui, |ui| {
                            ui.set_width(304.0);
                            ui.horizontal(|ui| {
                                let (dot, _) =
                                    ui.allocate_exact_size(Vec2::splat(5.0), Sense::hover());
                                ui.painter().circle_filled(dot.center(), 2.5, theme::GOLD);
                                ui.label(
                                    RichText::new(&toast.event.title)
                                        .size(11.5)
                                        .color(theme::GOLD)
                                        .strong(),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(theme::tiny("刚刚"));
                                });
                            });
                            ui.label(
                                RichText::new(&toast.event.message)
                                    .size(10.5)
                                    .color(theme::TEXT),
                            );
                        });
                });
            response.response.on_hover_cursor(egui::CursorIcon::Default);
        }
    }
}

fn threshold_widget(ui: &mut egui::Ui, editor: &mut AlertEditor) {
    let range = match (editor.scope, editor.metric) {
        (AlertScope::Market { .. }, AlertMetric::Price) => 0.01..=10_000_000.0,
        (AlertScope::Market { .. }, AlertMetric::DayChangePct) => -50.0..=50.0,
        (AlertScope::ComexBasis, _) => -1_000.0..=1_000.0,
        (AlertScope::SgePremium, _) => -50.0..=50.0,
        (AlertScope::UsdCny, _) => 0.01..=100.0,
    };
    let speed = match (editor.scope, editor.metric) {
        (AlertScope::Market { market }, AlertMetric::Price) => {
            if market.currency_symbol() == "$" {
                0.5
            } else {
                0.1
            }
        }
        (AlertScope::Market { .. }, AlertMetric::DayChangePct) => 0.05,
        (AlertScope::ComexBasis, _) => 0.1,
        (AlertScope::SgePremium, _) => 0.05,
        (AlertScope::UsdCny, _) => 0.0005,
    };
    let mut value = egui::DragValue::new(&mut editor.threshold)
        .range(range)
        .speed(speed)
        .max_decimals(match (editor.scope, editor.metric) {
            (AlertScope::UsdCny, _) => 4,
            (AlertScope::Market { .. }, AlertMetric::Price) => 2,
            _ => 2,
        });
    value = match (editor.scope, editor.metric) {
        (AlertScope::Market { market }, AlertMetric::Price) => {
            value.prefix(market.currency_symbol())
        }
        (AlertScope::Market { .. }, AlertMetric::DayChangePct) => value.suffix(" %"),
        (AlertScope::ComexBasis, _) => value.prefix("$").suffix(" /oz"),
        (AlertScope::SgePremium, _) => value.suffix(" %"),
        (AlertScope::UsdCny, _) => value,
    };
    ui.add(value);
}

fn threshold_unit(scope: AlertScope, metric: AlertMetric) -> &'static str {
    match (scope, metric) {
        (AlertScope::Market { market }, AlertMetric::Price) => market.unit(),
        (AlertScope::Market { .. }, AlertMetric::DayChangePct) => "%",
        (AlertScope::ComexBasis, _) => "美元/盎司",
        (AlertScope::SgePremium, _) => "%",
        (AlertScope::UsdCny, _) => "CNY",
    }
}

fn validate_editor(editor: &AlertEditor) -> bool {
    if !editor.threshold.is_finite() {
        return false;
    }
    match (editor.scope, editor.metric) {
        (AlertScope::Market { .. }, AlertMetric::Price) => editor.threshold > 0.0,
        (AlertScope::UsdCny, _) => editor.threshold > 0.0,
        _ => true,
    }
}

fn editor_validation_error(editor: &AlertEditor) -> String {
    if !editor.threshold.is_finite() {
        return "阈值必须是有效数字".to_owned();
    }
    if matches!(
        (editor.scope, editor.metric),
        (AlertScope::Market { .. }, AlertMetric::Price) | (AlertScope::UsdCny, _)
    ) && editor.threshold <= 0.0
    {
        return "价格 / 汇率阈值必须大于 0".to_owned();
    }
    "请检查告警参数".to_owned()
}

fn alert_card(
    ui: &mut egui::Ui,
    alert: &Alert,
    current: &str,
    confirm_delete: bool,
) -> Option<AlertAction> {
    let mut action = None;
    let mut enabled = alert.enabled;
    let status_color = if !alert.enabled {
        theme::FAINT
    } else if alert.last_condition {
        theme::GOLD
    } else {
        theme::GREEN
    };
    let status_text = if !alert.enabled {
        "已暂停"
    } else if alert.last_condition {
        "条件满足"
    } else {
        "监控中"
    };
    let border = if alert.enabled && alert.last_condition {
        theme::alpha(theme::GOLD, 110)
    } else {
        theme::BORDER_SOFT
    };
    Frame::new()
        .fill(if alert.enabled {
            theme::SURFACE_2
        } else {
            theme::alpha(theme::SURFACE_2, 130)
        })
        .stroke(Stroke::new(1.0_f32, border))
        .corner_radius(11)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.0, status_color);
                ui.label(
                    RichText::new(&alert.name)
                        .size(12.0)
                        .color(if alert.enabled {
                            theme::TEXT
                        } else {
                            theme::MUTED
                        })
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let response = switch(ui, &mut enabled);
                    if response.changed() {
                        action = Some(AlertAction::Toggle(alert.id.clone(), enabled));
                    }
                });
            });
            ui.label(
                RichText::new(alert.condition_text())
                    .size(10.5)
                    .color(if alert.enabled {
                        theme::MUTED
                    } else {
                        theme::FAINT
                    }),
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("当前 {current}")).size(10.5).color(
                    if alert.enabled {
                        theme::GOLD
                    } else {
                        theme::FAINT
                    },
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(theme::tiny(status_text));
                });
            });
            let last = alert
                .last_triggered
                .map(|ts| format!("上次触发 {}", ts.format("%m-%d %H:%M")))
                .unwrap_or_else(|| "尚未触发".to_owned());
            ui.horizontal(|ui| {
                ui.label(theme::tiny(last));
                if alert.repeat {
                    ui.label(theme::tiny("· 重复"));
                }
            });
            ui.add_space(2.0);
            if confirm_delete {
                ui.horizontal(|ui| {
                    ui.label(theme::tiny("确定删除这条告警？"));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::subtle_button("取消")).clicked() {
                            action = Some(AlertAction::CancelDelete);
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("删除").size(11.0).color(theme::RED),
                                )
                                .fill(theme::alpha(theme::RED, 25))
                                .corner_radius(7),
                            )
                            .clicked()
                        {
                            action = Some(AlertAction::Delete(alert.id.clone()));
                        }
                    });
                });
            } else {
                ui.horizontal(|ui| {
                    if ui.add(theme::ghost_button("编辑")).clicked() {
                        action = Some(AlertAction::Edit(alert.id.clone()));
                    }
                    if ui.add(theme::ghost_button("删除")).clicked() {
                        action = Some(AlertAction::AskDelete(alert.id.clone()));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if alert.trigger_count > 0 {
                            ui.label(theme::tiny(format!("已触发 {} 次", alert.trigger_count)));
                        }
                    });
                });
            }
        });
    action
}

fn session_alert_card(
    ui: &mut egui::Ui,
    alert: &SessionAlert,
    confirm_delete: bool,
    now: DateTime<chrono::FixedOffset>,
) -> Option<SessionAction> {
    let mut action = None;
    let mut enabled = alert.enabled;
    let open = alert.session.is_open(now);
    let status_color = if !alert.enabled {
        theme::FAINT
    } else if open {
        theme::GREEN
    } else {
        theme::MUTED
    };
    Frame::new()
        .fill(if alert.enabled {
            theme::SURFACE_2
        } else {
            theme::alpha(theme::SURFACE_2, 130)
        })
        .stroke(Stroke::new(1.0_f32, theme::BORDER_SOFT))
        .corner_radius(11)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.0, status_color);
                ui.label(
                    RichText::new(&alert.name)
                        .size(12.0)
                        .color(if alert.enabled {
                            theme::TEXT
                        } else {
                            theme::MUTED
                        })
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let response = switch(ui, &mut enabled);
                    if response.changed() {
                        action = Some(SessionAction::Toggle(alert.id.clone(), enabled));
                    }
                });
            });
            ui.label(
                RichText::new(format!(
                    "{} · {}（北京时间）",
                    alert.watch_text(),
                    alert.session.schedule_text()
                ))
                .size(10.5)
                .color(if alert.enabled {
                    theme::MUTED
                } else {
                    theme::FAINT
                }),
            );
            ui.horizontal(|ui| {
                let color = if open { theme::GREEN } else { theme::FAINT };
                ui.label(
                    RichText::new(if open { "当前开市" } else { "当前休市" })
                        .size(10.5)
                        .color(color),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(theme::tiny(alert.session.name()));
                });
            });
            ui.horizontal(|ui| {
                let last = alert
                    .last_triggered
                    .map(|ts| format!("上次触发 {}", ts.format("%m-%d %H:%M")))
                    .unwrap_or_else(|| "尚未触发".to_owned());
                ui.label(theme::tiny(last));
                if alert.trigger_count > 0 {
                    ui.label(theme::tiny(format!("· {} 次", alert.trigger_count)));
                }
            });
            ui.add_space(2.0);
            if confirm_delete {
                ui.horizontal(|ui| {
                    ui.label(theme::tiny("确定删除这条提醒？"));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::subtle_button("取消")).clicked() {
                            action = Some(SessionAction::CancelDelete);
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("删除").size(11.0).color(theme::RED),
                                )
                                .fill(theme::alpha(theme::RED, 25))
                                .corner_radius(7),
                            )
                            .clicked()
                        {
                            action = Some(SessionAction::Delete(alert.id.clone()));
                        }
                    });
                });
            } else {
                ui.horizontal(|ui| {
                    if ui.add(theme::ghost_button("编辑")).clicked() {
                        action = Some(SessionAction::Edit(alert.id.clone()));
                    }
                    if ui.add(theme::ghost_button("删除")).clicked() {
                        action = Some(SessionAction::AskDelete(alert.id.clone()));
                    }
                });
            }
        });
    action
}

fn switch(ui: &mut egui::Ui, value: &mut bool) -> egui::Response {
    let desired = Vec2::new(34.0, 19.0);
    let (rect, mut response) = ui.allocate_exact_size(desired, Sense::click());
    let how = ui.ctx().animate_bool_with_time(response.id, *value, 0.12);
    let bg = if *value {
        theme::GOLD
    } else {
        theme::SURFACE_3
    };
    ui.painter().rect_filled(rect, 10.0, bg);
    let x = rect.left() + 9.5 + (rect.width() - 19.0) * how;
    ui.painter()
        .circle_filled(Pos2::new(x, rect.center().y), 6.5, Color32::WHITE);
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn tab_button(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    let button = egui::Button::new(RichText::new(text).size(11.0).color(if selected {
        theme::GOLD
    } else {
        theme::MUTED
    }))
    .fill(if selected {
        theme::alpha(theme::GOLD, 18)
    } else {
        Color32::TRANSPARENT
    })
    .stroke(Stroke::new(
        1.0_f32,
        if selected {
            theme::alpha(theme::GOLD, 70)
        } else {
            theme::BORDER_SOFT
        },
    ))
    .corner_radius(7);
    ui.add(button).clicked()
}

fn template_button(ui: &mut egui::Ui, title: &str, subtitle: &str) -> bool {
    let response = Frame::new()
        .fill(theme::alpha(theme::SURFACE_2, 210))
        .stroke(Stroke::new(1.0_f32, theme::BORDER_SOFT))
        .corner_radius(9)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(title).size(10.5).color(theme::GOLD).strong());
                    ui.label(theme::tiny(subtitle));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(theme::tiny("＋"));
                });
            });
        })
        .response
        .interact(Sense::click());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn metric_card(ui: &mut egui::Ui, width: f32, label: &str, value: &str, sub: &str, color: Color32) {
    ui.allocate_ui_with_layout(Vec2::new(width, 88.0), Layout::top_down(Align::Min), |ui| {
        theme::card_tight().show(ui, |ui| {
            ui.set_width(width - 28.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(theme::tiny(label));
            ui.label(RichText::new(value).size(20.0).color(color).strong());
            ui.add(egui::Label::new(RichText::new(sub).size(8.5).color(theme::FAINT)).truncate());
        });
    });
}

fn area_mesh<FX, FY>(
    points: &[PricePoint],
    x_for: &FX,
    y_for: &FY,
    bottom: f32,
    color: Color32,
    alpha_top: u8,
) -> egui::Mesh
where
    FX: Fn(i64) -> f32,
    FY: Fn(f64) -> f32,
{
    let mut mesh = egui::Mesh::default();
    for (i, point) in points.iter().enumerate() {
        let x = x_for(point.ts);
        mesh.colored_vertex(
            Pos2::new(x, y_for(point.price)),
            theme::alpha(color, alpha_top),
        );
        mesh.colored_vertex(Pos2::new(x, bottom), theme::alpha(color, 0));
        if i > 0 {
            let base = (i as u32 - 1) * 2;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
    }
    mesh
}

fn moving_average(points: &[PricePoint], window_secs: i64) -> Vec<PricePoint> {
    let mut out = Vec::with_capacity(points.len());
    let mut start = 0usize;
    let mut sum = 0.0f64;
    for (index, point) in points.iter().enumerate() {
        sum += point.price;
        while start < index && points[start].ts < point.ts - window_secs {
            sum -= points[start].price;
            start += 1;
        }
        let count = (index + 1 - start).max(1) as f64;
        out.push(PricePoint::new(point.ts, sum / count));
    }
    out
}

fn draw_empty_chart(painter: &egui::Painter, plot: Rect) {
    for i in 0..=4 {
        let y = plot.top() + plot.height() * i as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0_f32, theme::alpha(theme::BORDER_SOFT, 100)),
        );
    }
    for i in 0..=3 {
        let x = plot.left() + plot.width() * i as f32 / 3.0;
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0_f32, theme::alpha(theme::BORDER_SOFT, 70)),
        );
    }
    painter.text(
        plot.center(),
        Align2::CENTER_CENTER,
        "等待行情数据",
        FontId::proportional(12.0),
        theme::MUTED,
    );
    painter.text(
        plot.center() + Vec2::new(0.0, 18.0),
        Align2::CENTER_CENTER,
        "数据接入后自动绘制走势",
        FontId::proportional(9.0),
        theme::FAINT,
    );
}

fn style_button(ui: &mut egui::Ui, style: ChartStyle, selected: bool) -> bool {
    let button = egui::Button::new(RichText::new(style.label()).size(9.5).color(if selected {
        theme::GOLD
    } else {
        theme::MUTED
    }))
    .fill(if selected {
        theme::alpha(theme::GOLD, 20)
    } else {
        Color32::TRANSPARENT
    })
    .stroke(Stroke::new(
        1.0_f32,
        if selected {
            theme::alpha(theme::GOLD, 70)
        } else {
            theme::BORDER_SOFT
        },
    ))
    .corner_radius(6);
    ui.add(button).on_hover_text(style.description()).clicked()
}

fn theme_money(value: f64) -> String {
    format!("${}", fmt_num(value, 2))
}

fn min_max(iter: impl Iterator<Item = f64>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for value in iter {
        if value.is_finite() {
            min = min.min(value);
            max = max.max(value);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        (0.0, 1.0)
    } else if (max - min).abs() < 1e-9 {
        (min - 1.0, max + 1.0)
    } else {
        (min, max)
    }
}

fn axis_price(value: f64, _market: MarketId, span: f64) -> String {
    if span >= 1000.0 {
        format!("{:.0}", value)
    } else if span >= 100.0 {
        format!("{:.1}", value)
    } else {
        format!("{:.2}", value)
    }
}

fn beijing_clock(ts: i64) -> String {
    beijing_tz()
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| "—".to_owned())
}

fn dashed_line(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    let stroke = Stroke::new(1.0_f32, color);
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1.0 {
        return;
    }
    let segments = (length / 6.0).floor().max(1.0) as usize;
    for i in 0..segments {
        let t0 = i as f32 / segments as f32;
        let t1 = ((i as f32 + 0.55) / segments as f32).min(1.0);
        painter.line_segment(
            [
                Pos2::new(from.x + dx * t0, from.y + dy * t0),
                Pos2::new(from.x + dx * t1, from.y + dy * t1),
            ],
            stroke,
        );
    }
}

fn session_range_open(now: DateTime<chrono::FixedOffset>, start_min: u32, end_min: u32) -> bool {
    let weekday = now.weekday();
    let mins = now.hour() * 60 + now.minute();
    let is_weekday = !matches!(weekday, chrono::Weekday::Sat | chrono::Weekday::Sun);
    if start_min < end_min {
        is_weekday && (start_min..=end_min).contains(&mins)
    } else {
        let evening = is_weekday && mins >= start_min;
        let morning = matches!(
            weekday,
            chrono::Weekday::Tue
                | chrono::Weekday::Wed
                | chrono::Weekday::Thu
                | chrono::Weekday::Fri
                | chrono::Weekday::Sat
        ) && mins <= end_min;
        evening || morning
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_average_uses_time_window_not_total_length() {
        let points: Vec<PricePoint> = (0..6)
            .map(|index| PricePoint::new(index * 60, (index + 1) as f64))
            .collect();
        let ma = moving_average(&points, 120);
        assert_eq!(ma.len(), 6);
        assert!((ma[0].price - 1.0).abs() < 1e-9);
        // 4, 5, 6 are within the last two minutes.
        assert!((ma[5].price - 5.0).abs() < 1e-9);
    }
}
