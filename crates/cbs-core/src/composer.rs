//! The chatbox composer: puts speech and the blocks together into one VRChat message,
//! makes room by priority, pages long speech and decides when to send.
//!
//! Live values (PC stats, media, lyrics, the foreground window) come from a [`LiveValues`]
//! source, the time from a [`Clock`], so the composer can be tested without either.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde_json::Value;

use crate::blocks::{self, keys_of};
use crate::catalog;
use crate::frames::{
    fmt_duration, scroll_windows, truncate, truncate16, typewriter_frames, window_text,
    with_icon, wrap_words, LIMIT,
};
use crate::jv::{str_list, to_f64, to_i64, to_str, truthy, Obj, ObjExt};
use crate::languages;
use crate::pyrandom::PyRandom;
use crate::pystr::{clen, head, slice, splitlines, split_ws, strip, ulen};
use crate::template::{fill, Values, GROUP, PH};

/// Monotonic and wall-clock time in seconds.
pub trait Clock: Send {
    fn mono(&self) -> f64;
    fn wall(&self) -> f64;
}

pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock(Instant::now())
    }
}

impl Clock for SystemClock {
    fn mono(&self) -> f64 {
        self.0.elapsed().as_secs_f64()
    }
    fn wall(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
}

/// Where the values come from that the composer can't work out itself.
pub trait LiveValues: Send {
    /// PC / VR / device values (sysinfo), already as text.
    fn stats(&mut self) -> Values {
        Values::new()
    }
    /// Media, lyrics and foreground-window values for these keys
    /// (title, artist, ..., audio_app, lyric_line, lyric_next, active_app, window_title).
    fn media(&mut self, _keys: &HashSet<String>, _cfg: &Obj) -> Result<Values, String> {
        Ok(Values::new())
    }
    /// When VRChat was started (wall clock), for `{playtime}`.
    fn vrchat_started(&mut self) -> Option<f64> {
        None
    }
    /// The language people around you speak (Earshot), as a code.
    fn room_language(&mut self) -> Option<String> {
        None
    }
    /// Which values are needed now (start / stop the watchers).
    fn configure(&mut self, _keys: &HashSet<String>, _cfg: &Obj) {}
    /// Keep reading everything while the block list is open.
    fn hold(&mut self, _on: bool) {}
}

/// A source without any live values.
pub struct NoValues;
impl LiveValues for NoValues {}

const CLOCK_STYLES: &[(&str, &str)] =
    &[("24h", "%H:%M"), ("24h_sec", "%H:%M:%S"), ("12h", "%I:%M %p"), ("12h_sec", "%I:%M:%S %p")];

pub fn tz_label(tz: &str) -> String {
    if tz.is_empty() || tz == "local" {
        return String::new();
    }
    tz.rsplit('/').next().unwrap_or("").replace('_', " ")
}

fn local_at(wall: f64) -> DateTime<Local> {
    let secs = wall.floor() as i64;
    let nanos = ((wall - wall.floor()) * 1e9) as u32;
    Local.timestamp_opt(secs, nanos).single().unwrap_or_else(Local::now)
}

/// strftime of the time at `wall` in a time zone ("local" or an IANA name).
fn strftime_in(tz: &str, wall: f64, fmt: &str) -> String {
    let local = local_at(wall);
    if tz.is_empty() || tz == "local" {
        return local.format(fmt).to_string();
    }
    match tz.parse::<chrono_tz::Tz>() {
        Ok(z) => local.with_timezone(&z).format(fmt).to_string(),
        Err(_) => local.format(fmt).to_string(),
    }
}

fn parse_target(raw: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M").ok().or_else(|| {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
    })
}

#[derive(Debug, Clone)]
struct SpeechItem {
    text: String,
    start: Option<f64>,
    frames: Option<Vec<(String, f64)>>,
    width: usize,
    /// live mode: end early for newer speech
    cut_at: Option<f64>,
    frame_idx: i64,
    frame_started: Option<f64>,
}

impl SpeechItem {
    fn new(text: &str, start: Option<f64>) -> Self {
        SpeechItem { text: text.to_string(), start, frames: None, width: 0, cut_at: None,
                     frame_idx: -1, frame_started: None }
    }

    fn shown_text(&self, el: &Obj) -> String {
        let limit = el.i_or("limit", 0);
        if limit != 0 { truncate(&self.text, limit.max(0) as usize) } else { self.text.clone() }
    }

    fn build(&mut self, el: &Obj, width: usize, min_interval: f64) {
        self.width = width;
        let text = self.shown_text(el);
        let mode = el.s("mode", "");
        let frames: Vec<(String, f64)> = if ["scroll", "marquee", "typewriter"].contains(&mode.as_str()) {
            let step_s = min_interval.max(el.f_or("scroll_seconds", min_interval));
            let step = el.i_or("scroll_words", 3);
            let wins: Vec<String> = match mode.as_str() {
                "scroll" => scroll_windows(&text, width, step.max(0) as usize),
                "marquee" => {
                    // once through, no loop for speech
                    let flat = split_ws(&text).join(" ");
                    let n = clen(&flat);
                    let stop = (n as i64 - width as i64 + 1).max(1) as usize;
                    let by = (step * 4).max(1) as usize;
                    let mut w: Vec<String> =
                        (0..stop).step_by(by).map(|i| slice(&flat, i, i + width)).collect();
                    let last = slice(&flat, n.saturating_sub(width), n);
                    if n > width && w.last() != Some(&last) {
                        w.push(last);
                    }
                    w
                }
                _ => {
                    let mut t = typewriter_frames(&text, width, step.max(0) as usize);
                    t.pop();
                    if t.is_empty() { vec![head(&text, width)] } else { t }
                }
            };
            wins.into_iter().map(|w| (w, step_s)).collect()
        } else {
            let fixed = el.f_or("page_seconds", 0.0);
            wrap_words(&text, width)
                .into_iter()
                .map(|p| {
                    let d = if fixed != 0.0 { fixed } else { (clen(&p) as f64 / 14.0).max(2.5) };
                    let d = min_interval.max(d);
                    (p, d)
                })
                .collect()
        };
        self.frames = Some(frames);
    }

    fn total(&self) -> f64 {
        self.frames.as_ref().map(|f| f.iter().map(|(_, d)| d).sum()).unwrap_or(0.0)
    }
}

/// What happened to a block in the last message (for the editor's preview).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartState {
    Shown,
    Empty,
    Dropped,
    Hidden,
    Idle,
    Off,
}

impl PartState {
    pub fn as_str(self) -> &'static str {
        match self {
            PartState::Shown => "shown",
            PartState::Empty => "empty",
            PartState::Dropped => "dropped",
            PartState::Hidden => "hidden",
            PartState::Idle => "idle",
            PartState::Off => "off",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub id: String,
    pub text: String,
    pub state: PartState,
}

pub type SendFn = Box<dyn FnMut(&str) -> Result<(), String> + Send>;
pub type PreviewFn = Box<dyn FnMut(&str, bool) + Send>;
pub type LogFn = Box<dyn FnMut(&str) + Send>;

const SPEECH_MARK: char = '\u{0}';

pub struct Composer {
    /// The normalised chatbox config (`settings.chatbox`).
    pub cfg: Obj,
    send: SendFn,
    log: LogFn,
    on_preview: PreviewFn,
    clock: Box<dyn Clock>,
    pub live: Box<dyn LiveValues>,
    app_started: f64,
    afk_since: Option<f64>,
    vrc_params: HashMap<String, Value>,
    speech: Vec<SpeechItem>,
    pub last_sent: String,
    pub last_sent_at: f64,
    last_preview: Option<String>,
    pub paused_sending: bool,
    pub last_dropped: Vec<String>,
    pub last_values: Values,
    pub last_parts: Vec<Part>,
    extra_keys: Vec<String>,
    logged: HashSet<String>,
}

impl Composer {
    pub fn new(chatbox_cfg: Option<&Value>, send: SendFn, log: LogFn, on_preview: PreviewFn,
               clock: Box<dyn Clock>, live: Box<dyn LiveValues>) -> Composer {
        let app_started = clock.wall();
        let mut c = Composer {
            cfg: blocks::normalize(chatbox_cfg),
            send,
            log,
            on_preview,
            clock,
            live,
            app_started,
            afk_since: None,
            vrc_params: HashMap::new(),
            speech: Vec::new(),
            last_sent: String::new(),
            last_sent_at: 0.0,
            last_preview: None,
            paused_sending: false,
            last_dropped: Vec::new(),
            last_values: Values::new(),
            last_parts: Vec::new(),
            extra_keys: Vec::new(),
            logged: HashSet::new(),
        };
        c.configure_watchers();
        c
    }

    fn min_interval(&self) -> f64 {
        self.cfg.f("min_interval", 1.2)
    }

    fn speech_element(&self) -> Obj {
        blocks::blocks(&self.cfg)
            .into_iter()
            .find(|b| b.s("type", "") == "speech")
            .cloned()
            .unwrap_or_default()
    }

    // ---- inputs -------------------------------------------------------------

    pub fn set_speech(&mut self, text: &str) {
        let text = strip(text).to_string();
        if text.is_empty() {
            return;
        }
        let now = self.clock.mono();
        let el = self.speech_element();
        let min_int = self.min_interval().max(1.0);
        self.expire(now);
        if self.speech.is_empty() {
            self.speech = vec![SpeechItem::new(&text, Some(now))];
        } else if el.s("flow", "live") == "queue" {
            self.speech.push(SpeechItem::new(&text, None));
            if self.speech.len() > 4 {
                // don't fall further behind
                let first = self.speech[0].clone();
                let rest = self.speech.split_off(self.speech.len() - 3);
                self.speech = std::iter::once(first).chain(rest).collect();
            }
        } else {
            // live: jump to the new sentence once the line on screen has been visible for one
            // VRChat update, so nothing disappears unseen
            let cur = &mut self.speech[0];
            match (cur.frames.is_some(), cur.frame_started) {
                (true, Some(fs)) => {
                    cur.cut_at = Some(now.max(fs + min_int));
                    let cur = cur.clone();
                    self.speech = vec![cur, SpeechItem::new(&text, None)];
                }
                _ => self.speech = vec![SpeechItem::new(&text, Some(now))],
            }
        }
    }

    /// Removes speech and sends an empty chatbox right away.
    pub fn clear(&mut self) {
        self.speech.clear();
        self.really_send(String::new());
    }

    /// Avatar parameters VRChat sends over OSC (VRMode, TrackingType, ...).
    pub fn set_vrc_param(&mut self, name: &str, value: Value) {
        self.vrc_params.insert(name.to_string(), value);
    }

    pub fn vrc_values(&self) -> Values {
        let p = &self.vrc_params;
        let mut v = Values::new();
        if let Some(m) = p.get("VRMode").and_then(to_i64) {
            v.insert("vrc_mode".into(), if m != 0 { "VR" } else { "Desktop" }.into());
        }
        if let Some(t) = p.get("TrackingType").and_then(to_i64) {
            let name = match t {
                1 => "Generic",
                2 => "Hands only",
                3 => "3-point",
                4 => "4-point (hip)",
                5 => "5-point",
                6 => "Full body",
                _ => "",
            };
            v.insert("vrc_tracking".into(), name.into());
        }
        let truthy_f = |k: &str| p.get(k).filter(|x| truthy(x)).and_then(to_f64);
        if let Some(h) = truthy_f("EyeHeightAsMeters") {
            v.insert("vrc_height".into(), format!("{h:.2} m"));
        }
        if let Some(s) = truthy_f("ScaleFactor") {
            v.insert("vrc_scale".into(), format!("{s:.2}x"));
        }
        if p.get("Seated").map(truthy).unwrap_or(false) {
            v.insert("vrc_seated".into(), "seated".into());
        }
        if p.get("Earmuffs").map(truthy).unwrap_or(false) {
            v.insert("vrc_earmuffs".into(), "earmuffs".into());
        }
        v
    }

    pub fn set_afk(&mut self, afk: bool) {
        if afk && self.afk_since.is_none() {
            self.afk_since = Some(self.clock.wall());
        } else if !afk {
            self.afk_since = None;
        }
    }

    /// Call after the GUI changed the config (it's normalised again).
    pub fn changed(&mut self) {
        let v = Value::Object(std::mem::take(&mut self.cfg));
        self.cfg = blocks::normalize(Some(&v));
        self.configure_watchers();
    }

    pub fn used_keys(&self, only_shown: bool) -> HashSet<String> {
        let mut out = HashSet::new();
        for b in blocks::blocks(&self.cfg) {
            if !only_shown || self.shown(b) {
                out.extend(keys_of(b));
            }
        }
        out
    }

    fn configure_watchers(&mut self) {
        let mut keys = self.used_keys(true);
        keys.extend(self.extra_keys.iter().cloned());
        self.live.configure(&keys, &self.cfg);
    }

    /// While the block list is on screen, read everything so it can show values.
    pub fn want_all_values(&mut self, on: bool) {
        self.extra_keys = if on {
            catalog::catalog().iter().map(|e| e.key.to_string()).collect()
        } else {
            Vec::new()
        };
        self.live.hold(on);
        self.configure_watchers();
    }

    fn shown(&self, b: &Obj) -> bool {
        let hidden = str_list(self.cfg.get("hidden_groups"));
        if b.s("type", "") == "speech" {
            return b.b("enabled", true) && b.b("visible", true)
                && !hidden.iter().any(|h| h == "speech");
        }
        if !b.b("visible", true) || self.cfg.b("hide_extras", false) {
            return false;
        }
        !(!hidden.is_empty() && blocks::groups_of(b).iter().any(|g| hidden.iter().any(|h| h == g)))
    }

    // ---- speech timing --------------------------------------------------------

    /// Drops finished speech items.
    fn expire(&mut self, now: f64) {
        let hold = self.speech_element().f("hold_seconds", 6.0);
        while !self.speech.is_empty() {
            let cur = &mut self.speech[0];
            let start = *cur.start.get_or_insert(now);
            if cur.frames.is_none() {
                return;
            }
            let end = cur.cut_at.unwrap_or(start + cur.total());
            if self.speech.len() > 1 {
                if now >= end {
                    self.speech.remove(0);
                    self.speech[0].start = Some(if now - end < 1.0 { end } else { now });
                    continue;
                }
                return;
            }
            if now >= end + hold {
                self.speech.remove(0);
                continue;
            }
            return;
        }
    }

    fn speech_text(&mut self, now: f64, width: usize) -> String {
        let el = self.speech_element();
        let min_interval = self.min_interval();
        self.expire(now);
        let Some(cur) = self.speech.first_mut() else { return String::new() };
        if cur.frames.is_none() {
            cur.build(&el, width, min_interval);
            if cur.start.is_none() {
                cur.start = Some(now);
            }
        }
        let frames = cur.frames.as_ref().unwrap();
        if frames.is_empty() {
            return String::new();
        }
        let mut t = now - cur.start.unwrap_or(now);
        let mut idx = frames.len() - 1;
        for (i, (_, dur)) in frames.iter().enumerate() {
            if t < *dur {
                idx = i;
                break;
            }
            t -= dur;
        }
        let text = frames[idx].0.clone();
        if idx as i64 != cur.frame_idx {
            cur.frame_idx = idx as i64;
            cur.frame_started = Some(now);
        }
        text
    }

    pub fn speech_active(&mut self, now: Option<f64>) -> bool {
        let now = match now {
            Some(n) if n != 0.0 => n,
            _ => self.clock.mono(),
        };
        self.expire(now);
        !self.speech.is_empty()
    }

    // ---- values ---------------------------------------------------------------

    /// All values the blocks use right now (without block options).
    pub fn values(&mut self, now_wall: f64, keys: &HashSet<String>) -> Result<Values, String> {
        let mut v = self.live.stats();
        v.extend(self.vrc_values());
        let media_keys = ["title", "artist", "album", "player", "site", "position", "duration",
                          "bar", "status", "audio_app", "lyric_line", "lyric_next",
                          "active_app", "window_title"];
        if media_keys.iter().any(|k| keys.contains(*k)) {
            let cfg = &self.cfg;
            v.extend(self.live.media(keys, cfg)?);
        }
        if let Some(since) = self.afk_since {
            v.insert("afk".into(), "AFK".into());
            v.insert("afk_time".into(), fmt_duration(now_wall - since, false));
        }
        if keys.contains("playtime") {
            if let Some(start) = self.live.vrchat_started() {
                v.insert("playtime".into(), fmt_duration(now_wall - start, false));
            }
        }
        v.insert("app_time".into(), fmt_duration(now_wall - self.app_started, false));
        if let Some(code) = self.live.room_language().filter(|c| !c.is_empty()) {
            v.insert("room_lang".into(), languages::name(&code));
        }
        v.insert("time".into(), strftime_in("local", now_wall, "%H:%M"));
        v.insert("date".into(), strftime_in("local", now_wall, "%d %b %Y"));
        v.insert("weekday".into(), strftime_in("local", now_wall, "%A"));
        Ok(v)
    }

    /// Adds the values that depend on this block's options (time zone, countdown, ...).
    pub fn block_values(&self, b: &Obj, base: &Values, now_wall: f64) -> Values {
        let ks = keys_of(b);
        let has = |k: &str| ks.iter().any(|x| x == k);
        let o = b.obj("opts").cloned().unwrap_or_default();
        let mut v = base.clone();
        if ["time", "date", "weekday", "tz"].iter().any(|k| has(k)) {
            let tz = o.s("timezone", "local");
            let style = o.s("style", "24h");
            let fmt = CLOCK_STYLES.iter().find(|(s, _)| *s == style).map(|(_, f)| *f).unwrap_or("%H:%M");
            let mut t = strftime_in(&tz, now_wall, fmt);
            if style.starts_with("12h") {
                t = t.trim_start_matches('0').to_string();
            }
            v.insert("time".into(), t);
            v.insert("date".into(), strftime_in(&tz, now_wall, "%d %b %Y"));
            v.insert("weekday".into(), strftime_in(&tz, now_wall, "%A"));
            v.insert("tz".into(), tz_label(&tz));
        }
        if ks.iter().any(|k| k.starts_with("countdown_")) {
            let raw = strip(&o.s("target", "")).to_string();
            if let Some(target) = parse_target(&raw) {
                let now = local_at(now_wall).naive_local();
                let left = (target - now).num_milliseconds() as f64 / 1000.0;
                let label = o.s("label", "");
                v.insert("countdown_label".into(), label.clone());
                v.insert("countdown_date".into(), target.format("%d %b %Y %H:%M").to_string());
                if left > 0.0 {
                    v.insert("countdown_left".into(), fmt_duration(left, left < 3600.0));
                } else {
                    v.insert("countdown_done".into(), if label.is_empty() { "now".into() } else { label });
                }
            }
        }
        let max_title = o.i_or("max_title", 0);
        if has("title") && max_title != 0 {
            if let Some(t) = v.get("title").filter(|t| !t.is_empty()).cloned() {
                v.insert("title".into(), truncate(&t, max_title.max(0) as usize));
            }
        }
        v
    }

    /// A text block: shows only when every value in it (outside `[ ]`) has something.
    /// Parts in `[ ]` disappear on their own when their value is empty.
    pub fn fill_block(template: &str, values: &Values) -> String {
        let outside_tpl = GROUP.replace_all(template, "");
        let outside: Vec<String> = PH.captures_iter(&outside_tpl).map(|c| c[1].to_string()).collect();
        if outside.iter().any(|k| {
            catalog::is_known(k) && strip(values.get(k).map(String::as_str).unwrap_or("")).is_empty()
        }) {
            return String::new();
        }
        let mut v = Values::new();
        for c in PH.captures_iter(template) {
            if catalog::is_known(&c[1]) {
                v.insert(c[1].to_string(), String::new());
            }
        }
        v.extend(values.iter().map(|(k, x)| (k.clone(), x.clone())));
        fill(template, &v)
    }

    pub fn render_block(&self, b: &Obj, base: &Values, now_wall: f64) -> String {
        let v = self.block_values(b, base, now_wall);
        let text = match b.s("type", "").as_str() {
            "value" => v.get(&b.s("key", "")).cloned().unwrap_or_default(),
            "text" => {
                let tpl = b.s("text", "");
                if PH.is_match(&tpl) { Self::fill_block(&tpl, &v) } else { tpl.replace("\\n", "\n") }
            }
            "rotating" => {
                let msgs: Vec<String> = splitlines(&b.s("messages", ""))
                    .iter()
                    .map(|m| strip(m).to_string())
                    .filter(|m| !m.is_empty())
                    .collect();
                if msgs.is_empty() {
                    return String::new();
                }
                let slot = (now_wall / b.f("interval", 10.0).max(2.0)).floor() as i64;
                let m = if b.b("random", false) {
                    &msgs[PyRandom::new(slot).randrange(msgs.len() as u32) as usize]
                } else {
                    &msgs[slot.rem_euclid(msgs.len() as i64) as usize]
                };
                Self::fill_block(m, &v)
            }
            _ => return String::new(),
        };
        window_text(&text, b, now_wall, self.min_interval())
    }

    // ---- building -------------------------------------------------------------

    /// Value blocks get a space between them; text blocks are joined as typed.
    fn join_line(items: &[(&Obj, &str)]) -> String {
        let mut out = String::new();
        let mut prev: Option<&Obj> = None;
        for (b, text) in items {
            if let Some(p) = prev {
                if b.s("type", "") != "text" && p.s("type", "") != "text" {
                    out.push(' ');
                }
            }
            out.push_str(text);
            prev = Some(b);
        }
        out
    }

    fn assemble(shown: &[&Obj], texts: &HashMap<String, String>, skip: &HashSet<String>) -> String {
        let mut lines: Vec<Vec<(&Obj, &str)>> = vec![Vec::new()];
        for b in shown {
            if b.s("type", "") == "newline" {
                lines.push(Vec::new());
                continue;
            }
            let id = b.s("id", "");
            if skip.contains(&id) {
                continue;
            }
            if let Some(t) = texts.get(&id).filter(|t| !t.is_empty()) {
                lines.last_mut().unwrap().push((b, t.as_str()));
            }
        }
        lines
            .iter()
            .map(|items| Self::join_line(items))
            .filter(|x| !strip(x).is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The chatbox message right now. `sample` shows that text as speech instead
    /// ("Show sample" in the editor).
    pub fn build(&mut self, now: Option<f64>, now_wall: Option<f64>, sample: Option<&str>) -> String {
        let (msg, parts) = self.build_parts(now, now_wall, sample);
        if sample.is_none() {
            self.last_parts = parts;
        }
        msg
    }

    fn log_once(&mut self, key: &str, msg: &str) {
        if self.logged.insert(key.to_string()) {
            (self.log)(msg);
        }
    }

    /// (message, one part per block in order).
    pub fn build_parts(&mut self, now: Option<f64>, now_wall: Option<f64>,
                       sample: Option<&str>) -> (String, Vec<Part>) {
        let now = now.unwrap_or_else(|| self.clock.mono());
        let now_wall = now_wall.unwrap_or_else(|| self.clock.wall());
        let allb: Vec<Obj> = blocks::blocks(&self.cfg).into_iter().cloned().collect();
        let shown: Vec<&Obj> = allb.iter().filter(|b| self.shown(b)).collect();
        let sp: Option<Obj> = shown.iter().find(|b| b.s("type", "") == "speech").map(|b| (*b).clone());
        let active = sp.is_some() && (sample.is_some() || self.speech_active(Some(now)));
        let mut keys: HashSet<String> = HashSet::new();
        for b in &shown {
            keys.extend(keys_of(b));
        }
        keys.extend(self.extra_keys.iter().cloned());
        let base = match self.values(now_wall, &keys) {
            Ok(v) => v,
            Err(ex) => {
                self.log_once("values", &format!("Chatbox values failed: {ex}"));
                self.live.stats()
            }
        };
        self.last_values = base.clone();

        let mut texts: HashMap<String, String> = HashMap::new();
        for b in &shown {
            let t = b.s("type", "");
            if t == "speech" || t == "newline" {
                continue;
            }
            texts.insert(b.s("id", ""), self.render_block(b, &base, now_wall));
        }

        let mut skip: HashSet<String> = HashSet::new();
        if active && self.cfg.b("speech_alone", false) {
            skip.extend(shown.iter().filter(|b| b.s("type", "") != "speech").map(|b| b.s("id", "")));
        }

        // how much room speech wants
        let mut need: i64 = 0;
        let mut pad: i64 = 0;
        let mut width_cfg: i64 = LIMIT as i64;
        let mut icon = String::new();
        let speech_prio;
        let sp_id = sp.as_ref().map(|b| b.s("id", "")).unwrap_or_default();
        if let (true, Some(sp)) = (active, sp.as_ref()) {
            width_cfg = sp.i_or("width", LIMIT as i64).min(LIMIT as i64);
            icon = strip(&sp.s("icon", "")).to_string();
            if !sp.get("icon").map(truthy).unwrap_or(false) {
                icon.clear();
            }
            pad = if icon.is_empty() { 0 } else { ulen(&icon) as i64 + 1 };
            need = if let Some(s) = sample {
                width_cfg.min((clen(s) as i64).max(1))
            } else {
                match self.speech.first() {
                    None => 1,
                    Some(cur) if cur.frames.is_some() => cur.width as i64, // keep the page size
                    Some(cur) => width_cfg.min((clen(&cur.shown_text(sp)) as i64).max(1)),
                }
            };
            let n = (need + pad).max(0) as usize;
            texts.insert(sp_id.clone(), std::iter::repeat_n(SPEECH_MARK, n).collect());
            speech_prio = sp.i("prio", 1);
        } else {
            speech_prio = 0;
        }

        // Make room: the highest priority number goes first (all blocks with it together).
        // 0 is never removed; while you talk only numbers above speech's make room.
        let mut dropped: Vec<String> = Vec::new();
        let candidate = |b: &Obj, skip: &HashSet<String>, texts: &HashMap<String, String>| {
            let t = b.s("type", "");
            let id = b.s("id", "");
            t != "speech" && t != "newline" && !skip.contains(&id)
                && texts.get(&id).map(|x| !x.is_empty()).unwrap_or(false)
                && b.i("prio", 2) > speech_prio
        };
        while ulen(&Self::assemble(&shown, &texts, &skip)) > LIMIT {
            let Some(top) = shown.iter().filter(|b| candidate(b, &skip, &texts)).map(|b| b.i("prio", 2)).max()
            else { break };
            for b in &shown {
                if candidate(b, &skip, &texts) && b.i("prio", 2) == top {
                    skip.insert(b.s("id", ""));
                    dropped.push(b.s("id", ""));
                }
            }
        }
        self.last_dropped = dropped.clone();

        let mut speech = String::new();
        if active {
            texts.insert(sp_id.clone(), SPEECH_MARK.to_string());
            let free = LIMIT as i64 - (ulen(&Self::assemble(&shown, &texts, &skip)) as i64 - 1);
            let want = if need > 1 { need } else { width_cfg };
            let width = 10i64.max(want.min(free - pad)) as usize;
            speech = match sample {
                Some(s) => with_icon(&icon, &head(s, width)),
                None => {
                    let t = self.speech_text(now, width);
                    with_icon(&icon, &t)
                }
            };
            texts.insert(sp_id.clone(), speech.clone());
        }
        let msg = strip(&Self::assemble(&shown, &texts, &skip)).to_string();

        let shown_ids: HashSet<String> = shown.iter().map(|b| b.s("id", "")).collect();
        let mut info = Vec::new();
        for b in &allb {
            let id = b.s("id", "");
            let t = b.s("type", "");
            let txt = texts.get(&id).cloned().unwrap_or_default();
            let (state, text) = if t == "speech" && !b.b("enabled", true) {
                (PartState::Off, String::new())
            } else if !shown_ids.contains(&id) {
                (PartState::Hidden, String::new())
            } else if t == "newline" {
                (PartState::Shown, String::new())
            } else if t == "speech" {
                if active { (PartState::Shown, speech.clone()) } else { (PartState::Idle, String::new()) }
            } else if skip.contains(&id) && !txt.is_empty() {
                (PartState::Dropped, txt)
            } else if !txt.is_empty() {
                (PartState::Shown, txt)
            } else {
                (PartState::Empty, String::new())
            };
            info.push(Part { id, text, state });
        }
        (truncate16(&msg, LIMIT), info)
    }

    // ---- sending --------------------------------------------------------------

    fn really_send(&mut self, msg: String) {
        if !self.paused_sending {
            if let Err(e) = (self.send)(&msg) {
                self.log_once("send", &format!("Chatbox send failed: {e}"));
            }
        }
        self.last_sent_at = self.clock.mono();
        self.last_preview = Some(msg.clone());
        (self.on_preview)(&msg, true);
        self.last_sent = msg;
    }

    /// One step of the send loop (run every 0.2 s). Returns the message if one was sent.
    pub fn tick(&mut self, now: Option<f64>) -> Option<String> {
        let now = now.unwrap_or_else(|| self.clock.mono());
        let msg = self.build(Some(now), None, None);
        if self.last_preview.as_deref() != Some(msg.as_str()) {
            self.last_preview = Some(msg.clone());
            (self.on_preview)(&msg, false);
        }
        let min_int = self.min_interval().max(1.0);
        let since = now - self.last_sent_at;
        if msg != self.last_sent && since >= min_int {
            self.really_send(msg.clone());
            return Some(msg);
        }
        let keep = self.cfg.f_or("keepalive", 0.0);
        if !msg.is_empty() && msg == self.last_sent && keep != 0.0 && since >= keep.max(min_int) {
            self.really_send(msg.clone());
            return Some(msg);
        }
        None
    }
}

/// `str(x)` for the GUI.
pub fn value_text(v: &Value) -> String {
    to_str(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    struct TestClock(Arc<Mutex<(f64, f64)>>);
    impl Clock for TestClock {
        fn mono(&self) -> f64 {
            self.0.lock().unwrap().0
        }
        fn wall(&self) -> f64 {
            self.0.lock().unwrap().1
        }
    }

    struct Stats;
    impl LiveValues for Stats {
        fn stats(&mut self) -> Values {
            [("cpu".to_string(), "12".to_string())].into_iter().collect()
        }
    }

    fn composer(cfg: Value) -> (Composer, Arc<Mutex<(f64, f64)>>, Arc<Mutex<Vec<String>>>) {
        let t = Arc::new(Mutex::new((100.0, 1_700_000_000.0)));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let s2 = sent.clone();
        let c = Composer::new(
            Some(&cfg),
            Box::new(move |m| {
                s2.lock().unwrap().push(m.to_string());
                Ok(())
            }),
            Box::new(|_| {}),
            Box::new(|_, _| {}),
            Box::new(TestClock(t.clone())),
            Box::new(Stats),
        );
        (c, t, sent)
    }

    #[test]
    fn values_and_layout() {
        let (mut c, _, _) = composer(json!({"blocks": [
            {"type": "speech", "id": "sp"},
            {"type": "text", "id": "a", "text": "CPU: {cpu}%"},
            {"type": "text", "id": "b", "text": "🎵 {title}"},
            {"type": "newline", "id": "nl"},
            {"type": "value", "id": "c", "key": "cpu"},
            {"type": "value", "id": "d", "key": "cpu"},
        ]}));
        assert_eq!(c.build(None, None, None), "CPU: 12%\n12 12");
        let st: Vec<&str> = c.last_parts.iter().map(|p| p.state.as_str()).collect();
        assert_eq!(st, vec!["idle", "shown", "empty", "shown", "shown", "shown"]);
    }

    #[test]
    fn speech_makes_room_and_pages() {
        let (mut c, t, sent) = composer(json!({"blocks": [
            {"type": "speech", "id": "sp"},
            {"type": "newline"},
            {"type": "text", "id": "lo", "text": "x".repeat(100), "prio": 3},
            {"type": "text", "id": "keep", "text": "KEEP", "prio": 0},
        ]}));
        c.set_speech(&"word ".repeat(40));
        let m = c.tick(None).unwrap();
        assert!(m.ends_with("\nKEEP"), "{m}");
        assert!(ulen(&m) <= 144);
        assert_eq!(c.last_dropped, vec!["lo".to_string()]);
        // after all pages and the hold time, the long block comes back
        t.lock().unwrap().0 += 200.0;
        let m = c.tick(None).unwrap();
        assert_eq!(m, format!("{}KEEP", "x".repeat(100)));
        assert_eq!(sent.lock().unwrap().len(), 2);
    }

    #[test]
    fn clock_styles_and_countdown() {
        let (c, _, _) = composer(json!({}));
        let b = blocks::new_block("text", json!({"text": "{time}", "opts": {"style": "12h", "timezone": "UTC"}})
            .as_object().unwrap().clone());
        let v = c.block_values(&b, &Values::new(), 1_700_000_000.0); // 2023-11-14 22:13:20 UTC
        assert_eq!(v["time"], "10:13 PM");
        assert_eq!(v["tz"], "UTC");
        let b = blocks::new_block("text", json!({"text": "{countdown_left}",
            "opts": {"target": "2099-01-01"}}).as_object().unwrap().clone());
        assert!(c.block_values(&b, &Values::new(), 1_700_000_000.0).contains_key("countdown_left"));
    }

    #[test]
    fn rotating_matches_python_slots() {
        let (c, _, _) = composer(json!({}));
        let b = blocks::new_block("rotating", json!({"messages": "a\nb\nc", "interval": 10.0,
            "random": true}).as_object().unwrap().clone());
        // Python: random.Random(0).randrange(3) == ... ; slot = now // 10
        let got = c.render_block(&b, &Values::new(), 5.0);
        let want = ["a", "b", "c"][PyRandom::new(0).randrange(3) as usize];
        assert_eq!(got, want);
        let b2 = blocks::new_block("rotating", json!({"messages": "a\nb\nc"}).as_object().unwrap().clone());
        assert_eq!(c.render_block(&b2, &Values::new(), 25.0), "c");
    }
}
