use api::GRAPHICS;
use api_ui_game::render::create_skin_container;
use client_containers::skins::SkinContainer;
use client_render_base::render::tee::RenderTee;
use client_ui::{connect2::user_data::UserData, events::UiEvents};
use game_base::connecting_log::{ConnectModes, ConnectingLog, ConnectingState};
use graphics::handles::canvas::canvas::GraphicsCanvasHandle;
use ui_base::types::{UiRenderPipe, UiState};
use ui_generic::traits::UiPageInterface;

pub struct Connecting {
    canvas_handle: GraphicsCanvasHandle,
    skin_container: SkinContainer,
    tee_render: RenderTee,
}

impl Default for Connecting {
    fn default() -> Self {
        Self::new()
    }
}

impl Connecting {
    pub fn new() -> Self {
        Self {
            canvas_handle: GRAPHICS.with(|graphics| graphics.canvas_handle.clone()),
            skin_container: create_skin_container(),
            tee_render: GRAPHICS.with(|graphics| RenderTee::new(graphics)),
        }
    }

    fn render_impl(
        &mut self,
        ui: &mut egui::Ui,
        pipe: &mut UiRenderPipe<()>,
        ui_state: &mut UiState,
    ) {
        let log = ConnectingLog::default();
        log.log("Downloading map");
        log.set_mode(ConnectModes::Connecting {
            addr: "127.0.0.1:8303".parse().unwrap(),
        });
        log.set_state(ConnectingState::DownloadingMap {
            map_name: "The new Tutorial".to_string(),
            downloaded_bytes: 501 * 1024,
            total_download_bytes: 1904 * 1024,
            download_speed_bytes_per_second: 42 * 1024,
        });
        client_ui::connect2::main_frame::render(
            ui,
            ui_state,
            &mut UiRenderPipe {
                cur_time: pipe.cur_time,
                user_data: &mut UserData {
                    log: &log,
                    config: &mut Default::default(),
                    events: &UiEvents::new(),
                    canvas_handle: &self.canvas_handle,
                    skin_container: &mut self.skin_container,
                    tee_render: &self.tee_render,
                },
            },
        );
    }
}

impl UiPageInterface<()> for Connecting {
    fn render(&mut self, ui: &mut egui::Ui, pipe: &mut UiRenderPipe<()>, ui_state: &mut UiState) {
        self.render_impl(ui, pipe, ui_state)
    }
}
