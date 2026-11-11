//! The port against the Python version: tests/fixtures/python.json holds what the Python
//! functions return for generated inputs (tools/make_fixtures.py writes it).

use cbs_core::filters::FilterSet;
use cbs_core::frames::*;
use cbs_core::routes::split_for_chatbox;
use cbs_core::settings::Settings;
use cbs_core::template::{fill, Values};
use serde_json::{json, Value};

fn fixtures() -> Value {
    serde_json::from_str(include_str!("fixtures/python.json")).unwrap()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

#[test]
fn frames_match() {
    let f = fixtures();
    let mut bad = 0;
    for c in f["frames"].as_array().unwrap() {
        let t = c["text"].as_str().unwrap();
        let w = c["width"].as_u64().unwrap() as usize;
        let step = (c["step"].as_u64().unwrap() as usize).max(1);
        let lines = c["lines"].as_u64().unwrap() as usize;
        let got: Vec<(&str, Value)> = vec![
            ("wrap", json!(wrap_words(t, w))),
            ("scroll", json!(scroll_windows(t, w, step))),
            ("ticker", json!(ticker_frames(t, w, step))),
            ("marquee", json!(marquee_frames(t, w, step))),
            ("bounce", json!(bounce_frames(t, w, step))),
            ("lines_f", json!(line_frames(t, w, lines))),
            ("typewriter", json!(typewriter_frames(t, w, step))),
            ("truncate", json!(truncate(t, w))),
            ("truncate16", json!(truncate16(t, w))),
            ("split", json!(split_for_chatbox(t, w))),
        ];
        for (name, g) in got {
            if g != c[name] {
                bad += 1;
                if bad < 8 {
                    eprintln!("{name} w={w} step={step} text={t:?}\n  rust:   {g}\n  python: {}", c[name]);
                }
            }
        }
    }
    assert_eq!(bad, 0);
}

#[test]
fn fill_matches() {
    for c in fixtures()["fill"].as_array().unwrap() {
        let vals: Values = c["values"].as_object().unwrap().iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        assert_eq!(fill(c["template"].as_str().unwrap(), &vals), c["out"].as_str().unwrap(),
                   "{c}");
    }
}

#[test]
fn durations_match() {
    for c in fixtures()["durations"].as_array().unwrap() {
        let got = fmt_duration(c["s"].as_f64().unwrap(), c["sec"].as_bool().unwrap());
        assert_eq!(got, c["out"].as_str().unwrap(), "{c}");
    }
}

#[test]
fn filters_match() {
    let mut s = Settings::default();
    s.replace_rules = strs_rules(json!([
        {"find": "vr chat", "replace": "VRChat"},
        {"find": "colour", "replace": "color"},
        {"find": r"(\d+) euros?", "replace": r"€\1", "regex": true, "whole_word": false}]));
    s.block_rules = strs_rules(json!([{"pattern": "naïve"}]));
    let fs = FilterSet::from_settings(&s);
    let mut bad = 0;
    for c in fixtures()["filters"].as_array().unwrap() {
        let o = fs.apply(c["text"].as_str().unwrap());
        let want = (c["out"].as_str().unwrap(), c["blocked"].as_bool().unwrap(),
                    c["reason"].as_str().unwrap());
        if (o.text.as_str(), o.blocked, o.reason.as_str()) != want {
            bad += 1;
            if bad < 8 {
                eprintln!("text={:?}\n  rust:   {:?} {} {:?}\n  python: {want:?}", c["text"],
                          o.text, o.blocked, o.reason);
            }
        }
    }
    assert_eq!(bad, 0);
}

fn strs_rules(v: Value) -> Vec<Value> {
    let _ = strs;
    v.as_array().unwrap().clone()
}
