//! Live market data adapters.
//!
//! Public endpoints are used so the app works without an API key. Sina Finance
//! provides the multi-market snapshot and intraday series; the Shanghai Gold
//! Exchange graph endpoint provides the Au99.99 intraday curve; ExchangeRate-API
//! supplies the USD/CNY reference rate. Gold-API is used as a fallback for spot
//! gold if the primary endpoint is unavailable.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::Deserialize;

use crate::model::{beijing_tz, now_beijing, FxRates, MarketId, PricePoint, Quote};

const SINA_QUOTE_URL: &str = "https://hq.sinajs.cn/list=hf_XAU,hf_GC,gds_AU9999,gds_AUTD,nf_AU0";
const SINA_HISTORY_URL: &str =
    "https://stock2.finance.sina.com.cn/futures/api/jsonp.php/var%20_{symbol}_/GlobalFuturesService.getGlobalFuturesMinLine?symbol={symbol}";
const SGE_CHART_URL: &str = "https://www.sge.com.cn/graph/quotations";
const GOLD_API_URL: &str = "https://api.gold-api.com/price/XAU";
const FX_URL: &str = "https://open.er-api.com/v6/latest/USD";

#[derive(Debug, Clone)]
pub struct FeedSnapshot {
    pub quotes: BTreeMap<MarketId, Quote>,
    pub fx: Option<FxRates>,
    pub fetched_at: DateTime<Local>,
    pub latency_ms: u128,
    pub source: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HistoryPayload {
    pub market: MarketId,
    pub points: Result<Vec<PricePoint>, String>,
}

#[derive(Debug, Clone)]
pub enum FeedEvent {
    Snapshot(FeedSnapshot),
    History(HistoryPayload),
}

pub enum WorkerCmd {
    Refresh,
    SetPollSecs(u64),
    FetchHistory(MarketId),
    Shutdown,
}

pub struct FeedWorker {
    pub events: Receiver<FeedEvent>,
    commands: Sender<WorkerCmd>,
}

impl FeedWorker {
    pub fn spawn(poll_secs: u64) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        thread::spawn(move || worker_loop(poll_secs, event_tx, cmd_rx));
        Self {
            events: event_rx,
            commands: cmd_tx,
        }
    }

    pub fn refresh_now(&self) {
        let _ = self.commands.send(WorkerCmd::Refresh);
    }

    pub fn set_poll_secs(&self, secs: u64) {
        let _ = self.commands.send(WorkerCmd::SetPollSecs(secs));
    }

    pub fn fetch_history(&self, market: MarketId) {
        let _ = self.commands.send(WorkerCmd::FetchHistory(market));
    }
}

impl Drop for FeedWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCmd::Shutdown);
    }
}

fn worker_loop(poll_secs: u64, tx: Sender<FeedEvent>, rx: Receiver<WorkerCmd>) {
    let client = match MarketClient::new() {
        Ok(client) => client,
        Err(err) => {
            let _ = tx.send(FeedEvent::Snapshot(FeedSnapshot {
                quotes: BTreeMap::new(),
                fx: None,
                fetched_at: Local::now(),
                latency_ms: 0,
                source: "初始化失败".to_owned(),
                warning: Some(err),
            }));
            return;
        }
    };
    let mut interval = Duration::from_secs(poll_secs.clamp(15, 300));
    let mut last_fetch = Instant::now()
        .checked_sub(interval)
        .unwrap_or_else(Instant::now);
    let mut fx: Option<FxRates> = None;
    let mut force = true;

    loop {
        if force || last_fetch.elapsed() >= interval {
            force = false;
            let snapshot = fetch_snapshot(&client, fx.as_ref());
            if let Some(new_fx) = snapshot.fx.clone() {
                fx = Some(new_fx);
            }
            if tx.send(FeedEvent::Snapshot(snapshot)).is_err() {
                break;
            }
            last_fetch = Instant::now();
        }

        match rx.recv_timeout(Duration::from_millis(150)) {
            Ok(WorkerCmd::Refresh) => force = true,
            Ok(WorkerCmd::SetPollSecs(secs)) => {
                interval = Duration::from_secs(secs.clamp(15, 300));
            }
            Ok(WorkerCmd::FetchHistory(market)) => {
                let _ = tx.send(FeedEvent::History(fetch_history(&client, market)));
            }
            Ok(WorkerCmd::Shutdown) => break,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn fetch_snapshot(client: &MarketClient, previous_fx: Option<&FxRates>) -> FeedSnapshot {
    let started = Instant::now();
    let mut warnings = Vec::new();
    let mut source_parts = Vec::new();

    // The two quote sources are fetched concurrently so that the fastest
    // timestamp wins without adding network latency to the refresh cycle.
    let (sina_result, spot_result) = std::thread::scope(|scope| {
        let sina = scope.spawn(|| client.fetch_sina_quotes());
        let spot = scope.spawn(|| client.fetch_gold_api());
        let sina = sina
            .join()
            .unwrap_or_else(|_| Err("行情线程异常".to_owned()));
        let spot = spot
            .join()
            .unwrap_or_else(|_| Err("现货线程异常".to_owned()));
        (sina, spot)
    });

    let mut quotes = match sina_result {
        Ok(quotes) => {
            source_parts.push("新浪财经");
            quotes
        }
        Err(err) => {
            warnings.push(format!("新浪行情不可用：{err}"));
            BTreeMap::new()
        }
    };

    let sina_london = quotes.get(&MarketId::London).cloned();
    match spot_result {
        Ok(spot) => {
            let prefer_spot = match (spot.as_of, sina_london.as_ref().and_then(|q| q.as_of)) {
                (Some(spot_ts), Some(sina_ts)) => spot_ts >= sina_ts,
                (Some(_), None) => true,
                (None, _) => sina_london.is_none(),
            };
            if prefer_spot {
                let mut merged = if let Some(sina) = &sina_london {
                    spot.merged_with(sina)
                } else {
                    spot
                };
                if sina_london.is_some() {
                    merged.source = "Gold-API 秒级 + 新浪统计".to_owned();
                }
                source_parts.push("Gold-API 秒级现货");
                quotes.insert(MarketId::London, merged);
            }
        }
        Err(err) => {
            if !quotes.contains_key(&MarketId::London) {
                warnings.push(format!("现货秒级源不可用：{err}"));
            }
        }
    }

    let needs_fx = previous_fx
        .map(|fx| (Local::now() - fx.updated_at).num_seconds() > 6 * 3600)
        .unwrap_or(true);
    let fx = if needs_fx {
        match client.fetch_fx() {
            Ok(rate) => {
                source_parts.push("ExchangeRate-API");
                Some(rate)
            }
            Err(err) => {
                warnings.push(format!("汇率源不可用：{err}"));
                previous_fx.cloned()
            }
        }
    } else {
        previous_fx.cloned()
    };

    if quotes.is_empty() {
        warnings.push("当前没有可用报价，正在自动重试".to_owned());
    } else {
        let missing: Vec<&str> = MarketId::ALL
            .iter()
            .filter(|market| !quotes.contains_key(market))
            .map(|market| market.short_name())
            .collect();
        if !missing.is_empty() {
            warnings.push(format!("{}行情暂缺", missing.join("、")));
        }
    }

    FeedSnapshot {
        quotes,
        fx,
        fetched_at: Local::now(),
        latency_ms: started.elapsed().as_millis(),
        source: if source_parts.is_empty() {
            "离线".to_owned()
        } else {
            source_parts.join(" · ")
        },
        warning: (!warnings.is_empty()).then(|| warnings.join("；")),
    }
}

fn fetch_history(client: &MarketClient, market: MarketId) -> HistoryPayload {
    let points = match market {
        MarketId::London | MarketId::NewYork => client.fetch_sina_history(market),
        MarketId::Shanghai => client.fetch_sge_history(),
        _ => Ok(Vec::new()),
    };
    HistoryPayload { market, points }
}

pub struct MarketClient {
    http: reqwest::blocking::Client,
}

impl MarketClient {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .user_agent("GoldWatcher/0.1 (+https://github.com/goldwatcher)")
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
        Ok(Self { http })
    }

    pub fn fetch_sina_quotes(&self) -> Result<BTreeMap<MarketId, Quote>, String> {
        let response = self
            .http
            .get(SINA_QUOTE_URL)
            .header("Referer", "https://finance.sina.com.cn/")
            .header("Accept", "*/*")
            .send()
            .map_err(|e| format!("网络错误：{e}"))?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let bytes = response.bytes().map_err(|e| format!("读取响应失败：{e}"))?;
        let text = String::from_utf8_lossy(&bytes);
        parse_sina_quotes(&text).map_err(|e| format!("解析失败：{e}"))
    }

    pub fn fetch_gold_api(&self) -> Result<Quote, String> {
        #[derive(Deserialize)]
        struct GoldApiResponse {
            price: f64,
            #[serde(default)]
            updated_at: String,
        }
        let response = self
            .http
            .get(GOLD_API_URL)
            .send()
            .map_err(|e| format!("网络错误：{e}"))?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let payload: GoldApiResponse = response.json().map_err(|e| format!("解析失败：{e}"))?;
        if !payload.price.is_finite() || payload.price <= 0.0 {
            return Err("返回价格无效".to_owned());
        }
        let mut quote = Quote::new(MarketId::London, payload.price);
        quote.source = "Gold-API 秒级".to_owned();
        if let Ok(dt) = DateTime::parse_from_rfc3339(&payload.updated_at) {
            quote.as_of = Some(dt.with_timezone(&beijing_tz()));
            quote.updated_at = dt.with_timezone(&Local);
        } else {
            quote.updated_at = Local::now();
        }
        Ok(quote)
    }

    pub fn fetch_fx(&self) -> Result<FxRates, String> {
        #[derive(Deserialize)]
        struct FxResponse {
            result: String,
            rates: HashMap<String, f64>,
            time_last_update_unix: Option<i64>,
        }
        let response = self
            .http
            .get(FX_URL)
            .send()
            .map_err(|e| format!("网络错误：{e}"))?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let payload: FxResponse = response.json().map_err(|e| format!("解析失败：{e}"))?;
        if payload.result != "success" {
            return Err("汇率服务返回失败状态".to_owned());
        }
        let get = |key: &str| {
            payload
                .rates
                .get(key)
                .copied()
                .filter(|v| v.is_finite() && *v > 0.0)
        };
        let usd_cny = get("CNY").ok_or_else(|| "缺少 CNY 汇率".to_owned())?;
        Ok(FxRates {
            usd_cny,
            usd_jpy: get("JPY").unwrap_or(0.0),
            usd_inr: get("INR").unwrap_or(0.0),
            updated_at: payload
                .time_last_update_unix
                .and_then(|ts| Local.timestamp_opt(ts, 0).single())
                .unwrap_or_else(Local::now),
            source: "ExchangeRate-API".to_owned(),
        })
    }

    pub fn fetch_sina_history(&self, market: MarketId) -> Result<Vec<PricePoint>, String> {
        let symbol = market
            .history_symbol()
            .ok_or_else(|| "该市场没有分钟线接口".to_owned())?;
        let url = SINA_HISTORY_URL.replace("{symbol}", symbol);
        let response = self
            .http
            .get(url)
            .header("Referer", "https://finance.sina.com.cn/")
            .send()
            .map_err(|e| format!("网络错误：{e}"))?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let text = response.text().map_err(|e| format!("读取响应失败：{e}"))?;
        let json = json_object_slice(&text).ok_or_else(|| "响应不是有效 JSONP".to_owned())?;
        parse_sina_history(json)
    }

    pub fn fetch_sge_history(&self) -> Result<Vec<PricePoint>, String> {
        let response = self
            .http
            .get(SGE_CHART_URL)
            .header("Referer", "https://www.sge.com.cn/")
            .send()
            .map_err(|e| format!("网络错误：{e}"))?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let payload: SgeChart = response.json().map_err(|e| format!("解析失败：{e}"))?;
        parse_sge_history(&payload)
    }
}

fn json_object_slice(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

pub fn parse_sina_quotes(text: &str) -> Result<BTreeMap<MarketId, Quote>, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some((symbol, rest)) = line.split_once('=') else {
            continue;
        };
        let symbol = symbol
            .trim()
            .trim_start_matches("var ")
            .trim_start_matches("hq_str_");
        let market = match symbol {
            "hf_XAU" => MarketId::London,
            "hf_GC" => MarketId::NewYork,
            "gds_AU9999" => MarketId::Shanghai,
            "gds_AUTD" => MarketId::ShanghaiTd,
            "nf_AU0" => MarketId::Shfe,
            _ => continue,
        };
        let Some(first_quote) = rest.find('"') else {
            continue;
        };
        let Some(last_quote) = rest.rfind('"') else {
            continue;
        };
        if last_quote <= first_quote {
            continue;
        }
        let payload = &rest[first_quote + 1..last_quote];
        if payload.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = payload.split(',').collect();
        match market {
            MarketId::Shfe => {
                let price = field(&fields, 8);
                let prev = field(&fields, 5);
                let open = field(&fields, 2);
                let high = field(&fields, 3);
                let low = field(&fields, 4);
                let Some(price) = price.filter(|p| *p > 0.0) else {
                    continue;
                };
                let mut q = Quote::new(market, price);
                q.prev_close = prev.unwrap_or(f64::NAN);
                q.open = open.unwrap_or(f64::NAN);
                q.high = high.unwrap_or(f64::NAN);
                q.low = low.unwrap_or(f64::NAN);
                q.bid = field(&fields, 6);
                q.ask = field(&fields, 7);
                q.volume = field(&fields, 14);
                q.quote_time = fields.get(1).and_then(|raw| parse_hhmmss(raw));
                q.as_of = parse_source_datetime(
                    fields.get(17).copied().unwrap_or_default(),
                    q.quote_time.as_deref().unwrap_or_default(),
                );
                q.source = "新浪财经 · SHFE".to_owned();
                out.insert(market, q);
            }
            _ => {
                let price = field(&fields, 0);
                let Some(price) = price.filter(|p| *p > 0.0) else {
                    continue;
                };
                let mut q = Quote::new(market, price);
                match market {
                    MarketId::London => {
                        q.prev_close = field(&fields, 7).unwrap_or(f64::NAN);
                        q.open = field(&fields, 8).unwrap_or(f64::NAN);
                        q.bid = field(&fields, 1);
                        q.ask = field(&fields, 2);
                    }
                    MarketId::NewYork => {
                        q.prev_close = field(&fields, 8).unwrap_or(f64::NAN);
                        q.open = field(&fields, 3).unwrap_or(f64::NAN);
                        q.bid = field(&fields, 1);
                        q.ask = field(&fields, 2);
                        q.volume = field(&fields, 14);
                    }
                    MarketId::Shanghai | MarketId::ShanghaiTd => {
                        q.prev_close = field(&fields, 7).unwrap_or(f64::NAN);
                        q.open = field(&fields, 3).unwrap_or(f64::NAN);
                        q.bid = field(&fields, 2);
                        q.ask = field(&fields, 2);
                        q.volume = field(&fields, 9);
                    }
                    MarketId::Shfe => unreachable!(),
                }
                q.high = field(&fields, 4).unwrap_or(f64::NAN);
                q.low = field(&fields, 5).unwrap_or(f64::NAN);
                q.quote_time = fields
                    .get(6)
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty());
                q.as_of = parse_source_datetime(
                    fields.get(12).copied().unwrap_or_default(),
                    q.quote_time.as_deref().unwrap_or_default(),
                );
                q.source = "新浪财经".to_owned();
                out.insert(market, q);
            }
        }
    }
    if out.is_empty() {
        Err("没有解析到任何行情".to_owned())
    } else {
        Ok(out)
    }
}

fn parse_source_datetime(date: &str, time: &str) -> Option<DateTime<chrono::FixedOffset>> {
    let date = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let time = NaiveTime::parse_from_str(time.trim(), "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(time.trim(), "%H:%M"))
        .ok()?;
    beijing_tz()
        .from_local_datetime(&date.and_time(time))
        .single()
}

fn field(fields: &[&str], index: usize) -> Option<f64> {
    fields
        .get(index)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

fn parse_hhmmss(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.len() != 6 || !raw.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}:{}:{}", &raw[0..2], &raw[2..4], &raw[4..6]))
}

pub fn parse_sina_history(json: &str) -> Result<Vec<PricePoint>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("分钟线 JSON 解析失败：{e}"))?;
    let rows = value
        .get("minLine_1d")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "缺少 minLine_1d 字段".to_owned())?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 6 {
            continue;
        }
        let price = fields
            .get(1)
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0);
        let Some(price) = price else {
            continue;
        };
        let datetime = fields
            .iter()
            .rev()
            .filter_map(|v| v.as_str())
            .find(|v| v.len() >= 19 && v.contains(' '))
            .and_then(|v| NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S").ok());
        let ts = if let Some(dt) = datetime {
            beijing_tz()
                .from_local_datetime(&dt)
                .single()
                .map(|dt| dt.timestamp())
        } else {
            let date = fields
                .first()
                .and_then(|v| v.as_str())
                .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
            let time = fields
                .get(5)
                .and_then(|v| v.as_str())
                .and_then(|v| NaiveTime::parse_from_str(v, "%H:%M").ok());
            match (date, time) {
                (Some(date), Some(time)) => beijing_tz()
                    .from_local_datetime(&date.and_time(time))
                    .single()
                    .map(|dt| dt.timestamp()),
                _ => None,
            }
        };
        if let Some(ts) = ts {
            out.push(PricePoint::new(ts, price));
        }
    }
    out.sort_by_key(|p| p.ts);
    out.dedup_by_key(|p| p.ts);
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct SgeChart {
    #[serde(default)]
    times: Vec<String>,
    #[serde(default)]
    data: Vec<String>,
    #[serde(default)]
    delaystr: String,
}

fn parse_sge_history(chart: &SgeChart) -> Result<Vec<PricePoint>, String> {
    let count = chart.times.len().min(chart.data.len());
    if count == 0 {
        return Ok(Vec::new());
    }
    let last_date =
        parse_sge_delay_date(&chart.delaystr).unwrap_or_else(|| now_beijing().date_naive());
    let mut out = Vec::with_capacity(count);
    let mut date = last_date;

    // The series starts on the previous evening and ends in the current day
    // session. Walk it backwards so a 23:xx -> 00:xx transition rolls the date.
    let mut next_time: Option<NaiveTime> = None;
    for i in (0..count).rev() {
        let Some(time) = NaiveTime::parse_from_str(chart.times[i].trim(), "%H:%M")
            .ok()
            .or_else(|| NaiveTime::parse_from_str(chart.times[i].trim(), "%H:%M:%S").ok())
        else {
            continue;
        };
        if let Some(next) = next_time {
            if time > next {
                date -= chrono::Duration::days(1);
            }
        }
        next_time = Some(time);
        let Some(price) = chart.data[i].trim().parse::<f64>().ok() else {
            continue;
        };
        if !price.is_finite() || price <= 0.0 {
            continue;
        }
        if let Some(dt) = beijing_tz()
            .from_local_datetime(&date.and_time(time))
            .single()
        {
            out.push(PricePoint::new(dt.timestamp(), price));
        }
    }
    out.reverse();
    out.sort_by_key(|p| p.ts);
    out.dedup_by_key(|p| p.ts);
    Ok(out)
}

fn parse_sge_delay_date(raw: &str) -> Option<NaiveDate> {
    NaiveDateTime::parse_from_str(raw.trim(), "%Y年%m月%d日 %H:%M:%S")
        .ok()
        .map(|dt| dt.date())
        .or_else(|| {
            NaiveDate::parse_from_str(raw.trim().split(' ').next().unwrap_or(""), "%Y年%m月%d日")
                .ok()
        })
}

/// Convert a USD/oz price to CNY/g, used in tests and diagnostics.
#[cfg(test)]
pub fn usd_oz_to_cny_g(usd_oz: f64, usd_cny: f64) -> f64 {
    usd_oz * usd_cny / crate::model::TROY_OUNCE_GRAMS
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINA_FIXTURE: &str = r#"var hq_str_hf_XAU="4023.77,4024.260,4023.77,4024.12,4032.56,3986.78,16:52:00,4024.26,4025.41,0,0,0,2026-10-23,伦敦金（现货黄金）";
var hq_str_hf_GC="4039.523,,4038.300,4038.600,4047.800,4000.900,16:52:39,4060.400,4042.200,0,3,2,2026-10-23,纽约黄金,0";
var hq_str_gds_AU9999="864.00,0,863.16,864.00,883.98,856.01,15:29:58,880.30,874.50,1396300,100.00,695.00,2026-10-23,沪金99";
var hq_str_gds_AUTD="863.35,0,863.30,863.80,880.04,856.24,15:30:03,879.66,872.80,56720,2.00,10.00,2026-10-23,黄金延期";
var hq_str_nf_AU0="黄金连续,150000,880.000,881.000,858.080,864.360,864.160,864.360,864.360,869.200,886.380,1,1,198578.000,286554,沪,黄金,2026-10-23,1";
"#;

    #[test]
    fn parses_all_sina_markets() {
        let quotes = parse_sina_quotes(SINA_FIXTURE).expect("fixture should parse");
        assert_eq!(quotes.len(), 5);
        let london = &quotes[&MarketId::London];
        assert!((london.price - 4023.77).abs() < 1e-9);
        assert!((london.prev_close - 4024.26).abs() < 1e-9);
        assert!((london.change_pct().unwrap()).abs() < 0.02);
        assert_eq!(
            london
                .as_of
                .expect("source timestamp")
                .format("%H:%M")
                .to_string(),
            "16:52"
        );
        let ny = &quotes[&MarketId::NewYork];
        assert!((ny.open - 4038.6).abs() < 1e-9);
        assert!((ny.prev_close - 4042.2).abs() < 1e-9);
        let sh = &quotes[&MarketId::Shanghai];
        assert!((sh.price - 864.0).abs() < 1e-9);
        let shfe = &quotes[&MarketId::Shfe];
        assert!((shfe.price - 864.36).abs() < 1e-9);
        assert_eq!(shfe.quote_time.as_deref(), Some("15:00:00"));
    }

    #[test]
    fn parses_sina_minute_history() {
        let json = r#"{"minLine_1d":[["2026-10-23","4024.260","LIFFE","","06:00","4024.210","0","0","4024.620","2026-10-23 06:00:00"],["06:01","4025.59","0","0","4025.087","2026-10-23 06:01:00"],["06:02","4024.96","0","0","4025.046","2026-10-23 06:02:00"]]}"#;
        let points = parse_sina_history(json).expect("history should parse");
        assert_eq!(points.len(), 3);
        assert!((points[1].price - 4025.59).abs() < 1e-9);
        assert!(points[2].ts - points[1].ts == 60);
    }

    #[test]
    fn parses_sge_history_across_midnight() {
        let chart = SgeChart {
            times: vec![
                "23:59".into(),
                "00:00".into(),
                "09:00".into(),
                "15:30".into(),
            ],
            data: vec![
                "870.0".into(),
                "871.0".into(),
                "874.0".into(),
                "864.0".into(),
            ],
            delaystr: "2026年10月23日 15:45:00".into(),
        };
        let points = parse_sge_history(&chart).expect("SGE history should parse");
        assert_eq!(points.len(), 4);
        let first = points[0].time_beijing();
        let second = points[1].time_beijing();
        assert_eq!(first.date_naive().to_string(), "2026-10-22");
        assert_eq!(second.date_naive().to_string(), "2026-10-23");
        assert_eq!(
            points[3].time_beijing().format("%H:%M").to_string(),
            "15:30"
        );
    }

    #[test]
    fn converts_ounces_to_grams() {
        assert!((usd_oz_to_cny_g(4025.0, 6.688) - 865.45).abs() < 0.2);
    }
}
