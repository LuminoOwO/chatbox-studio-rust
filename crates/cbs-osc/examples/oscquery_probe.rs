//! Manual check against other mDNS implementations:
//! `cargo run -p cbs-osc --example oscquery_probe -- advertise 9123` or `-- find`.

use std::time::Duration;

use cbs_osc::oscquery::{OscQueryService, VrchatFinder};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("advertise") => {
            let port = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(9123);
            let mut s = OscQueryService::new("ChatboxStudio", port);
            let http = s.start().expect("start");
            println!("advertising {} osc={port} http={http}", s.name);
            std::thread::sleep(Duration::from_secs(args.get(3).and_then(|x| x.parse().ok()).unwrap_or(8)));
        }
        _ => {
            let _f = VrchatFinder::start(|p| println!("vrchat port: {p:?}")).expect("start");
            std::thread::sleep(Duration::from_secs(8));
        }
    }
}
