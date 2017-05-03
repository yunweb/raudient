#![crate_type = "lib"]
#![crate_name = "raudient"]

extern crate gtk;
extern crate glib;
extern crate chrono;
extern crate encoding;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate env_logger;


mod constant;
mod model;
mod demons;
mod message;
mod util;
mod chat_window;
pub mod app;
