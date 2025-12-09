// TODO

use serde::{Deserialize, Serialize};

use crate::debug::hot_reload::HotReloadConfig;

pub struct CliConnection {}

pub struct CliSender {}

pub struct CliReceiver {}
impl CliConnection {
    pub fn connect(_config: HotReloadConfig) -> Self {
        todo!()
    }

    pub fn split(self) -> (CliSender, CliReceiver) {
        todo!()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum CliEvent {
    HotReload { binary: Vec<u8> },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PanicReport {}

#[derive(Debug, Serialize, Deserialize)]
pub enum AppEvent {
    Crashed {},
}
