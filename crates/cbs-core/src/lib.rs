//! Chatbox Studio core: settings, languages, routes, filters, chatbox blocks and the
//! composer. No I/O beyond reading and writing the settings file.

mod catalog_data;

pub mod blocks;
pub mod catalog;
pub mod composer;
pub mod filters;
pub mod frames;
pub mod jv;
pub mod languages;
pub mod legacy;
pub mod paths;
pub mod pyrandom;
pub mod pystr;
pub mod routes;
pub mod settings;
pub mod template;
