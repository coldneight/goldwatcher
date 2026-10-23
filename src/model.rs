//! Core domain types for GoldWatcher.
//!
//! Prices are stored in the native unit of each market. Cross-market metrics
//! (COMEX basis and Shanghai premium) are derived from the London spot quote
//! and the latest USD/CNY reference rate.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Datelike, FixedOffset, Local, TimeZone, Timelike, Utc, Weekday};
use serde::{Deserialize, Serialize};

pub const TROY_OUNCE_GRAMS: f64 = 31.103_476_8;
pub const BEIJING_OFFSET_SECS: i32 = 8 * 3600;

pub fn beijing_tz() -> FixedOffset {
    FixedOffset::east_opt(BEIJING_OFFSET_SECS).expect("valid UTC+8 offset")
}

pub fn now_beijing() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&beijing_tz())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MarketId {
    London,
    NewYork,
    Shanghai,
    ShanghaiTd,
    Shfe,
}

impl MarketId {
    pub const ALL: [Self; 5] = [
        Self::London,
        Self::NewYork,
        Self::Shanghai,
        Self::ShanghaiTd,
        Self::Shfe,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::London => "伦敦金现货",
            Self::NewYork => "COMEX 黄金",
            Self::Shanghai => "上海金 Au99.99",
            Self::ShanghaiTd => "黄金 T+D",
            Self::Shfe => "沪金主力",
        }
    }

    pub const fn short_name(self) -> &'static str {
        match self {
            Self::London => "伦敦金",
            Self::NewYork => "纽约金",
            Self::Shanghai => "上海金",
            Self::ShanghaiTd => "黄金 T+D",
            Self::Shfe => "沪金主力",
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::London => "XAU/USD",
            Self::NewYork => "GC",
            Self::Shanghai => "Au99.99",
            Self::ShanghaiTd => "Au(T+D)",
            Self::Shfe => "AU",
        }
    }

    pub const fn region(self) -> &'static str {
        match self {
            Self::London => "伦敦 · LBMA",
            Self::NewYork => "纽约 · COMEX",
            Self::Shanghai | Self::ShanghaiTd => "上海 · SGE",
            Self::Shfe => "上海 · SHFE",
        }
    }

    pub const fn unit(self) -> &'static str {
        match self {
            Self::London | Self::NewYork => "美元/盎司",
            _ => "元/克",
        }
    }

    pub const fn currency_symbol(self) -> &'static str {
        match self {
            Self::London | Self::NewYork => "$",
            _ => "¥",
        }
    }

    pub const fn decimals(self) -> usize {
        2
    }

    /// Sina global-futures symbol used by the intraday chart endpoint.
    pub const fn history_symbol(self) -> Option<&'static str> {
        match self {
            Self::London => Some("XAU"),
            Self::NewYork => Some("GC"),
            _ => None,
        }
    }

    pub fn is_open(self, now: DateTime<FixedOffset>) -> bool {
        let weekday = now.weekday();
        let mins = now.hour() * 60 + now.minute();
        match self {
            Self::London => !matches!(weekday, Weekday::Sat | Weekday::Sun),
            Self::NewYork => {
                // CME Globex is effectively open Sunday evening through Friday
                // evening, with a one-hour daily maintenance window.
                let maintenance = (5 * 60..6 * 60).contains(&mins);
                match weekday {
                    Weekday::Sat => mins < 5 * 60,
                    Weekday::Sun => false,
                    _ => !maintenance,
                }
            }
            Self::Shanghai | Self::ShanghaiTd | Self::Shfe => {
                let day_session = (9 * 60..=15 * 60 + 30).contains(&mins);
                let night_session = mins >= 20 * 60;
                let early_session = mins <= 2 * 60 + 30;
                match weekday {
                    Weekday::Sat => early_session,
                    Weekday::Sun => false,
                    _ => day_session || night_session || early_session,
                }
            }
        }
    }

    pub fn fmt_price(self, value: f64) -> String {
        format!(
            "{}{}",
            self.currency_symbol(),
            fmt_num(value, self.decimals())
        )
    }

    pub fn fmt_signed(self, value: f64) -> String {
        let sign = if value > 0.0 {
            "+"
        } else if value < 0.0 {
            "-"
        } else {
            ""
        };
        format!(
            "{}{}{}",
            sign,
            self.currency_symbol(),
            fmt_num(value.abs(), self.decimals())
        )
    }
}

impl fmt::Display for MarketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SessionId {
    London,
    NewYork,
    ShanghaiDay,
    ShanghaiNight,
    TokyoDay,
    TokyoNight,
    India,
    Dubai,
}

impl SessionId {
    pub const ALL: [Self; 8] = [
        Self::London,
        Self::NewYork,
        Self::ShanghaiDay,
        Self::ShanghaiNight,
        Self::TokyoDay,
        Self::TokyoNight,
        Self::India,
        Self::Dubai,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::London => "伦敦盘",
            Self::NewYork => "纽约 COMEX",
            Self::ShanghaiDay => "上海日盘",
            Self::ShanghaiNight => "上海夜盘",
            Self::TokyoDay => "东京日盘",
            Self::TokyoNight => "东京夜盘",
            Self::India => "印度 MCX",
            Self::Dubai => "迪拜 DGCX",
        }
    }

    pub const fn schedule_text(self) -> &'static str {
        match self {
            Self::London => "15:00–24:00",
            Self::NewYork => "06:00–次日 05:00",
            Self::ShanghaiDay => "09:00–15:30",
            Self::ShanghaiNight => "20:00–次日 02:30",
            Self::TokyoDay => "08:00–14:15",
            Self::TokyoNight => "16:00–次日 05:00",
            Self::India => "11:30–次日 02:00",
            Self::Dubai => "11:00–次日 03:00",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::London => "伦敦金市交易时段（北京时间，参考）",
            Self::NewYork => "COMEX 电子盘交易时段（北京时间，参考）",
            Self::ShanghaiDay => "上海黄金交易所日盘",
            Self::ShanghaiNight => "上海黄金交易所夜盘",
            Self::TokyoDay => "东京 TOCOM 日盘",
            Self::TokyoNight => "东京 TOCOM 夜盘",
            Self::India => "印度 MCX 黄金交易时段",
            Self::Dubai => "迪拜 DGCX 黄金交易时段",
        }
    }

    pub fn is_open(self, now: DateTime<FixedOffset>) -> bool {
        let (start, end) = match self {
            Self::London => (15 * 60, 24 * 60),
            Self::NewYork => (6 * 60, 5 * 60),
            Self::ShanghaiDay => (9 * 60, 15 * 60 + 30),
            Self::ShanghaiNight => (20 * 60, 2 * 60 + 30),
            Self::TokyoDay => (8 * 60, 14 * 60 + 15),
            Self::TokyoNight => (16 * 60, 5 * 60),
            Self::India => (11 * 60 + 30, 2 * 60),
            Self::Dubai => (11 * 60, 3 * 60),
        };
        weekday_window_open(now, start, end)
    }
}

fn weekday_window_open(now: DateTime<FixedOffset>, start_min: u32, end_min: u32) -> bool {
    let weekday = now.weekday();
    let mins = now.hour() * 60 + now.minute();
    let weekday_open = !matches!(weekday, Weekday::Sat | Weekday::Sun);
    if start_min < end_min {
        weekday_open && mins >= start_min && mins < end_min
    } else {
        let evening = weekday_open && mins >= start_min;
        let morning = matches!(
            weekday,
            Weekday::Tue | Weekday::Wed | Weekday::Thu | Weekday::Fri | Weekday::Sat
        ) && mins < end_min;
        evening || morning
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAlert {
    pub id: String,
    pub name: String,
    pub session: SessionId,
    pub watch_open: bool,
    pub watch_close: bool,
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub notify: bool,
    #[serde(default = "default_true")]
    pub flash: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Local>>,
    #[serde(default)]
    pub last_triggered: Option<DateTime<Local>>,
    #[serde(default)]
    pub trigger_count: u64,
}

impl SessionAlert {
    pub fn new(session: SessionId, watch_open: bool, watch_close: bool) -> Self {
        let mut parts = Vec::new();
        if watch_open {
            parts.push("开盘");
        }
        if watch_close {
            parts.push("收盘");
        }
        Self {
            id: new_id(),
            name: format!("{} {}", session.name(), parts.join(" / ")),
            session,
            watch_open,
            watch_close,
            enabled: true,
            notify: true,
            flash: true,
            created_at: Some(Local::now()),
            last_triggered: None,
            trigger_count: 0,
        }
    }

    pub fn watch_text(&self) -> String {
        let mut parts = Vec::new();
        if self.watch_open {
            parts.push("开盘");
        }
        if self.watch_close {
            parts.push("收盘");
        }
        if parts.is_empty() {
            "未选择事件".to_owned()
        } else {
            parts.join(" + ")
        }
    }

    pub fn trigger_title(&self, opened: bool) -> String {
        format!(
            "{} · {}提醒",
            self.session.name(),
            if opened { "开盘" } else { "收盘" }
        )
    }

    pub fn trigger_message(&self, opened: bool, now: DateTime<FixedOffset>) -> String {
        format!(
            "{}已{}（北京时间 {}）",
            self.session.description(),
            if opened { "开盘" } else { "收盘" },
            now.format("%H:%M")
        )
    }

    pub fn watches(&self, opened: bool) -> bool {
        if opened {
            self.watch_open
        } else {
            self.watch_close
        }
    }
}

#[derive(Debug, Clone)]
pub struct Quote {
    #[allow(dead_code)]
    pub market: MarketId,
    pub price: f64,
    pub prev_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub volume: Option<f64>,
    pub quote_time: Option<String>,
    /// Timestamp reported by the data source, converted to UTC+8.
    pub as_of: Option<DateTime<FixedOffset>>,
    pub updated_at: DateTime<Local>,
    pub source: String,
}

impl Quote {
    pub fn new(market: MarketId, price: f64) -> Self {
        Self {
            market,
            price,
            prev_close: f64::NAN,
            open: f64::NAN,
            high: f64::NAN,
            low: f64::NAN,
            bid: None,
            ask: None,
            volume: None,
            quote_time: None,
            as_of: None,
            updated_at: Local::now(),
            source: String::new(),
        }
    }

    pub fn change(&self) -> Option<f64> {
        (self.prev_close.is_finite() && self.prev_close > 0.0)
            .then_some(self.price - self.prev_close)
    }

    pub fn change_pct(&self) -> Option<f64> {
        let prev = self.prev_close;
        (prev.is_finite() && prev > 0.0).then_some((self.price - prev) / prev * 100.0)
    }

    pub fn range_pct(&self) -> Option<f64> {
        (self.high.is_finite() && self.low.is_finite() && self.prev_close > 0.0)
            .then_some((self.high - self.low) / self.prev_close * 100.0)
    }

    pub fn is_valid(&self) -> bool {
        self.price.is_finite() && self.price > 0.0
    }

    pub fn age_secs(&self) -> i64 {
        (Local::now() - self.updated_at).num_seconds().max(0)
    }

    /// Age of the quote according to the source timestamp, not the fetch time.
    pub fn quote_age_secs(&self) -> Option<i64> {
        self.as_of
            .map(|ts| (now_beijing() - ts).num_seconds().max(0))
    }

    pub fn freshness_text(&self) -> String {
        match self.quote_age_secs() {
            Some(age) if age < 2 => "实时".to_owned(),
            Some(age) if age < 90 => format!("{age} 秒前"),
            Some(age) if age < 3_600 => format!("{} 分钟前", age / 60),
            Some(age) => format!("{} 小时前", age / 3_600),
            None => "时间未知".to_owned(),
        }
    }

    pub fn merged_with(self, fallback: &Quote) -> Quote {
        let mut out = self;
        if !out.prev_close.is_finite() || out.prev_close <= 0.0 {
            out.prev_close = fallback.prev_close;
        }
        if !out.open.is_finite() || out.open <= 0.0 {
            out.open = fallback.open;
        }
        if !out.high.is_finite() || out.high <= 0.0 {
            out.high = fallback.high;
        }
        if !out.low.is_finite() || out.low <= 0.0 {
            out.low = fallback.low;
        }
        if out.quote_time.is_none() {
            out.quote_time = fallback.quote_time.clone();
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct FxRates {
    pub usd_cny: f64,
    #[allow(dead_code)]
    pub usd_jpy: f64,
    #[allow(dead_code)]
    pub usd_inr: f64,
    pub updated_at: DateTime<Local>,
    pub source: String,
}

impl FxRates {
    pub fn is_valid(&self) -> bool {
        self.usd_cny.is_finite() && self.usd_cny > 0.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct MarketMetrics {
    pub usd_cny: Option<f64>,
    pub comex_basis: Option<f64>,
    pub comex_basis_pct: Option<f64>,
    pub sge_premium_cny: Option<f64>,
    pub sge_premium_pct: Option<f64>,
}

impl MarketMetrics {
    pub fn derive(quotes: &BTreeMap<MarketId, Quote>, fx: Option<&FxRates>) -> Self {
        let mut out = Self::default();
        let london = quotes.get(&MarketId::London);
        let usd_cny = fx.filter(|f| f.is_valid()).map(|f| f.usd_cny);
        out.usd_cny = usd_cny;

        if let (Some(london), Some(new_york)) = (london, quotes.get(&MarketId::NewYork)) {
            if london.is_valid() && new_york.is_valid() {
                let basis = new_york.price - london.price;
                out.comex_basis = Some(basis);
                out.comex_basis_pct = Some(basis / london.price * 100.0);
            }
        }

        if let (Some(london), Some(shanghai), Some(rate)) =
            (london, quotes.get(&MarketId::Shanghai), usd_cny)
        {
            if london.is_valid() && shanghai.is_valid() {
                let international_cny_g = london.price * rate / TROY_OUNCE_GRAMS;
                if international_cny_g > 0.0 {
                    out.sge_premium_cny = Some(shanghai.price - international_cny_g);
                    out.sge_premium_pct =
                        Some((shanghai.price - international_cny_g) / international_cny_g * 100.0);
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChartWindow {
    Hour1,
    #[default]
    Hour4,
    Today,
}

impl ChartWindow {
    pub const ALL: [Self; 3] = [Self::Hour1, Self::Hour4, Self::Today];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hour1 => "1 小时",
            Self::Hour4 => "4 小时",
            Self::Today => "今日",
        }
    }

    pub const fn seconds(self) -> i64 {
        match self {
            Self::Hour1 => 3_600,
            Self::Hour4 => 4 * 3_600,
            Self::Today => 86_400,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertScope {
    Market { market: MarketId },
    ComexBasis,
    SgePremium,
    UsdCny,
}

impl Default for AlertScope {
    fn default() -> Self {
        Self::Market {
            market: MarketId::London,
        }
    }
}

impl AlertScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Market { market } => market.name(),
            Self::ComexBasis => "COMEX 基差",
            Self::SgePremium => "上海金溢价",
            Self::UsdCny => "美元 / 人民币",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Market { market } => market.short_name(),
            Self::ComexBasis => "COMEX 基差",
            Self::SgePremium => "上海金溢价",
            Self::UsdCny => "USD/CNY",
        }
    }

    pub fn is_market(self) -> bool {
        matches!(self, Self::Market { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AlertMetric {
    #[default]
    Price,
    DayChangePct,
}

impl AlertMetric {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Price => "价格",
            Self::DayChangePct => "日涨跌幅",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Above,
    Below,
}

impl Direction {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Above => "高于 / 上破",
            Self::Below => "低于 / 跌破",
        }
    }

    pub fn matches(self, value: f64, threshold: f64) -> bool {
        match self {
            Self::Above => value >= threshold,
            Self::Below => value <= threshold,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub name: String,
    pub scope: AlertScope,
    #[serde(default)]
    pub metric: AlertMetric,
    pub direction: Direction,
    pub threshold: f64,
    pub enabled: bool,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
    #[serde(default = "default_true")]
    pub notify: bool,
    #[serde(default = "default_true")]
    pub flash: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Local>>,
    #[serde(default)]
    pub last_triggered: Option<DateTime<Local>>,
    #[serde(default)]
    pub last_triggered_value: Option<f64>,
    #[serde(default)]
    pub trigger_count: u64,
    #[serde(default)]
    pub last_condition: bool,
    #[serde(default)]
    pub last_value: Option<f64>,
}

fn default_cooldown() -> u64 {
    300
}
fn default_true() -> bool {
    true
}

impl Alert {
    pub fn new(
        name: impl Into<String>,
        scope: AlertScope,
        metric: AlertMetric,
        direction: Direction,
        threshold: f64,
    ) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            scope,
            metric,
            direction,
            threshold,
            enabled: true,
            repeat: false,
            cooldown_secs: 300,
            notify: true,
            flash: true,
            created_at: Some(Local::now()),
            last_triggered: None,
            last_triggered_value: None,
            trigger_count: 0,
            last_condition: false,
            last_value: None,
        }
    }

    pub fn threshold_text(&self) -> String {
        format_threshold(self.scope, self.metric, self.threshold)
    }

    pub fn condition_text(&self) -> String {
        let op = match self.direction {
            Direction::Above => "≥",
            Direction::Below => "≤",
        };
        format!("{} {} {}", self.metric_label(), op, self.threshold_text())
    }

    pub fn metric_label(&self) -> &'static str {
        match self.scope {
            AlertScope::Market { .. } => self.metric.label(),
            AlertScope::ComexBasis => "基差",
            AlertScope::SgePremium => "溢价",
            AlertScope::UsdCny => "汇率",
        }
    }

    pub fn value(
        &self,
        quotes: &BTreeMap<MarketId, Quote>,
        metrics: &MarketMetrics,
    ) -> Option<f64> {
        match self.scope {
            AlertScope::Market { market } => {
                let quote = quotes.get(&market)?;
                if !quote.is_valid() {
                    return None;
                }
                match self.metric {
                    AlertMetric::Price => Some(quote.price),
                    AlertMetric::DayChangePct => quote.change_pct(),
                }
            }
            AlertScope::ComexBasis => metrics.comex_basis,
            AlertScope::SgePremium => metrics.sge_premium_pct,
            AlertScope::UsdCny => metrics.usd_cny,
        }
    }

    pub fn current_text(
        &self,
        quotes: &BTreeMap<MarketId, Quote>,
        metrics: &MarketMetrics,
    ) -> String {
        self.value(quotes, metrics)
            .map(|v| format_value(self.scope, self.metric, v))
            .unwrap_or_else(|| "—".to_owned())
    }

    pub fn cooldown_ready(&self, now: DateTime<Local>) -> bool {
        match self.last_triggered {
            None => true,
            Some(previous) => {
                let elapsed = (now - previous).num_seconds().max(0) as u64;
                elapsed >= self.cooldown_secs
            }
        }
    }

    /// Returns a human-readable one-line reminder when this alert fires.
    pub fn trigger_message(&self, value: f64) -> String {
        let current = format_value(self.scope, self.metric, value);
        let relation = match (self.direction, self.metric) {
            (Direction::Above, AlertMetric::Price) => "已上破提醒值",
            (Direction::Below, AlertMetric::Price) => "已跌破提醒值",
            (Direction::Above, _) => "已高于提醒值",
            (Direction::Below, _) => "已低于提醒值",
        };
        let threshold = format_threshold(self.scope, self.metric, self.threshold);
        match self.scope {
            AlertScope::Market { .. } if self.metric == AlertMetric::Price => {
                format!("当前 {current}，{relation} {threshold}")
            }
            _ => format!("当前 {current}，{relation} {threshold}"),
        }
        .trim()
        .to_owned()
    }
}

fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("a{:x}", nanos)
}

pub fn format_threshold(scope: AlertScope, metric: AlertMetric, value: f64) -> String {
    match (scope, metric) {
        (AlertScope::Market { market }, AlertMetric::Price) => market.fmt_price(value),
        (AlertScope::Market { .. }, AlertMetric::DayChangePct) => {
            format!("{:+.2}%", value)
        }
        (AlertScope::ComexBasis, _) => format!(
            "{}{}",
            if value >= 0.0 { "+" } else { "-" },
            market_dollar(value.abs())
        ),
        (AlertScope::SgePremium, _) => format!("{:+.2}%", value),
        (AlertScope::UsdCny, _) => format!("{:.4}", value),
    }
}

fn market_dollar(value: f64) -> String {
    format!("${}", fmt_num(value, 2))
}

pub fn format_value(scope: AlertScope, metric: AlertMetric, value: f64) -> String {
    format_threshold(scope, metric, value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: String,
    pub alert_id: String,
    pub title: String,
    pub message: String,
    pub triggered_at: DateTime<Local>,
    pub value: f64,
    #[serde(default)]
    pub read: bool,
}

impl AlertEvent {
    pub fn new_session(alert: &SessionAlert, opened: bool) -> Self {
        let now = now_beijing();
        Self {
            id: new_id(),
            alert_id: alert.id.clone(),
            title: alert.trigger_title(opened),
            message: alert.trigger_message(opened, now),
            triggered_at: Local::now(),
            value: if opened { 1.0 } else { 0.0 },
            read: false,
        }
    }

    pub fn new(alert: &Alert, message: String, value: f64) -> Self {
        Self {
            id: new_id(),
            alert_id: alert.id.clone(),
            title: format!("{} · {}", alert.scope.short_label(), alert.metric_label()),
            message,
            triggered_at: Local::now(),
            value,
            read: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub poll_secs: u64,
    pub notifications: bool,
    pub flash: bool,
    pub auto_start: bool,
    pub selected_market: MarketId,
    pub chart_window: ChartWindow,
    pub alerts: Vec<Alert>,
    #[serde(default)]
    pub session_alerts: Vec<SessionAlert>,
    pub events: Vec<AlertEvent>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            poll_secs: 15,
            notifications: true,
            flash: true,
            auto_start: false,
            selected_market: MarketId::London,
            chart_window: ChartWindow::Hour4,
            alerts: Vec::new(),
            session_alerts: Vec::new(),
            events: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PricePoint {
    pub ts: i64,
    pub price: f64,
}

impl PricePoint {
    pub fn new(ts: i64, price: f64) -> Self {
        Self { ts, price }
    }

    #[allow(dead_code)]
    pub fn time_beijing(&self) -> DateTime<FixedOffset> {
        beijing_tz()
            .timestamp_opt(self.ts, 0)
            .single()
            .unwrap_or_else(|| beijing_tz().timestamp_opt(0, 0).unwrap())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HistoryStore {
    pub data: BTreeMap<MarketId, Vec<PricePoint>>,
}

impl HistoryStore {
    pub fn push(&mut self, market: MarketId, point: PricePoint) {
        if !point.price.is_finite() || point.price <= 0.0 {
            return;
        }
        let entry = self.data.entry(market).or_default();
        let should_push = entry
            .last()
            .map(|last| point.ts - last.ts >= 30 || (point.price - last.price).abs() > f64::EPSILON)
            .unwrap_or(true);
        if should_push {
            entry.push(point);
        }
        let cutoff = Utc::now().timestamp() - 7 * 86_400;
        entry.retain(|p| p.ts >= cutoff);
        if entry.len() > 10_080 {
            let drain = entry.len() - 10_080;
            entry.drain(0..drain);
        }
    }

    pub fn seed(&mut self, market: MarketId, points: Vec<PricePoint>) {
        if points.is_empty() {
            return;
        }
        let entry = self.data.entry(market).or_default();
        entry.extend(points);
        entry.sort_by_key(|p| p.ts);
        entry.dedup_by_key(|p| p.ts);
        let cutoff = Utc::now().timestamp() - 7 * 86_400;
        entry.retain(|p| p.ts >= cutoff);
        if entry.len() > 10_080 {
            let drain = entry.len() - 10_080;
            entry.drain(0..drain);
        }
    }

    pub fn points(&self, market: MarketId, window: ChartWindow) -> Vec<PricePoint> {
        let cutoff = Utc::now().timestamp() - window.seconds();
        self.data
            .get(&market)
            .map(|points| points.iter().filter(|p| p.ts >= cutoff).cloned().collect())
            .unwrap_or_default()
    }
}

pub fn fmt_num(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    let raw = format!("{:.*}", decimals, value.abs());
    let (int_part, frac_part) = raw.split_once('.').unwrap_or((raw.as_str(), ""));
    let mut grouped = String::with_capacity(int_part.len() + int_part.len() / 3 + 4);
    for (i, ch) in int_part.chars().enumerate() {
        if i > 0 && (int_part.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let sign = if value < 0.0 { "-" } else { "" };
    if decimals == 0 {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac_part}")
    }
}

pub fn fmt_pct(value: f64) -> String {
    if value.is_finite() {
        format!("{:+.2}%", value)
    } else {
        "—".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_grouped_numbers() {
        assert_eq!(fmt_num(4023.77, 2), "4,023.77");
        assert_eq!(fmt_num(-12.5, 2), "-12.50");
        assert_eq!(fmt_num(0.0, 4), "0.0000");
    }

    #[test]
    fn derives_cross_market_metrics() {
        let mut quotes = BTreeMap::new();
        let mut london = Quote::new(MarketId::London, 4_000.0);
        london.prev_close = 3_980.0;
        let mut ny = Quote::new(MarketId::NewYork, 4_006.0);
        ny.prev_close = 3_990.0;
        let mut sh = Quote::new(MarketId::Shanghai, 870.0);
        sh.prev_close = 865.0;
        quotes.insert(MarketId::London, london);
        quotes.insert(MarketId::NewYork, ny);
        quotes.insert(MarketId::Shanghai, sh);
        let fx = FxRates {
            usd_cny: 6.7,
            usd_jpy: 150.0,
            usd_inr: 83.0,
            updated_at: Local::now(),
            source: "test".into(),
        };
        let m = MarketMetrics::derive(&quotes, Some(&fx));
        assert!((m.comex_basis.unwrap() - 6.0).abs() < 1e-9);
        let intl = 4_000.0 * 6.7 / TROY_OUNCE_GRAMS;
        assert!((m.sge_premium_cny.unwrap() - (870.0 - intl)).abs() < 1e-9);
    }

    #[test]
    fn alert_direction_is_inclusive() {
        assert!(Direction::Above.matches(10.0, 10.0));
        assert!(Direction::Below.matches(10.0, 10.0));
        assert!(!Direction::Above.matches(9.99, 10.0));
    }

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<FixedOffset> {
        beijing_tz()
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("valid test time")
    }

    #[test]
    fn session_windows_match_beijing_schedule() {
        // 2026-10-23 is a Friday.
        assert!(SessionId::London.is_open(at(2026, 10, 23, 15, 0)));
        assert!(!SessionId::London.is_open(at(2026, 10, 23, 14, 59)));
        assert!(!SessionId::London.is_open(at(2026, 10, 24, 12, 0)));

        assert!(SessionId::ShanghaiDay.is_open(at(2026, 10, 23, 9, 0)));
        assert!(!SessionId::ShanghaiDay.is_open(at(2026, 10, 23, 15, 30)));
        assert!(SessionId::ShanghaiNight.is_open(at(2026, 10, 23, 20, 0)));
        assert!(!SessionId::ShanghaiNight.is_open(at(2026, 10, 24, 2, 30)));
        assert!(SessionId::ShanghaiNight.is_open(at(2026, 10, 24, 2, 29)));

        assert!(SessionId::NewYork.is_open(at(2026, 10, 23, 6, 0)));
        assert!(!SessionId::NewYork.is_open(at(2026, 10, 24, 5, 0)));
        assert!(!SessionId::NewYork.is_open(at(2026, 10, 25, 12, 0)));
    }

    #[test]
    fn session_alert_message_mentions_open_close() {
        let alert = SessionAlert::new(SessionId::NewYork, true, true);
        let opened = alert.trigger_message(true, at(2026, 10, 23, 6, 0));
        assert!(opened.contains("开盘"));
        assert!(opened.contains("06:00"));
        let closed = alert.trigger_message(false, at(2026, 10, 24, 5, 0));
        assert!(closed.contains("收盘"));
        assert!(closed.contains("05:00"));
    }

    #[test]
    fn market_sessions_do_not_panic() {
        let now = now_beijing();
        for m in MarketId::ALL {
            let _ = m.is_open(now);
        }
    }
}
