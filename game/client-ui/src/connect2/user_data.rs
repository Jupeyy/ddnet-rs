use client_containers::skins::SkinContainer;
use client_render_base::render::tee::RenderTee;
use game_base::connecting_log::ConnectingLog;
use game_config::config::Config;
use graphics::handles::canvas::canvas::GraphicsCanvasHandle;

use crate::events::UiEvents;

pub struct UserData<'a> {
    pub log: &'a ConnectingLog,
    pub config: &'a mut Config,
    pub events: &'a UiEvents,

    pub canvas_handle: &'a GraphicsCanvasHandle,
    pub skin_container: &'a mut SkinContainer,
    pub tee_render: &'a RenderTee,
}
