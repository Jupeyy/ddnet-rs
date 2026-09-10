use serde::{Deserialize, Serialize};

use crate::server_browser::ServerBrowserInfo;

/// Backend ports refer to the host serving the authenticated S2S endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerInfo {
    pub browser_info: ServerBrowserInfo,
    pub game_port_v4: u16,
    pub game_port_v6: u16,
}
