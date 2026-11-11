//! Chatbox Studio: VRChat OSC (chatbox out; mute, AFK and avatar parameters in), OSCQuery
//! advertising and finding the port VRChat really listens on.

pub mod bridge;
pub mod dns;
pub mod mdns;
pub mod osc;
pub mod oscquery;

pub use bridge::{Events, OscBridge};
