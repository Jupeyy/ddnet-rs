use std::time::Duration;

use client_render_base::render::{
    animation::AnimState,
    default_anim::{base_anim, run_right_anim},
    tee::{TeeRenderHands, TeeRenderInfo, TeeRenderSkinColor},
};
use egui::{
    Color32, Grid, Layout, Mesh, ProgressBar, RichText, ScrollArea, Separator, Stroke,
    text::LayoutJob,
};

use egui_extras::{Size, StripBuilder};
use game_base::connecting_log::{ConnectModes, ConnectingState};
use game_interface::types::{
    character_info::NetworkSkinInfo, render::character::TeeEye, resource_key::ResourceKey,
};
use graphics_types::rendering::State;
use math::math::vector::vec2;
use tracing::instrument;
use ui_base::{
    types::{UiRenderPipe, UiState},
    utils::add_margins,
};

use crate::{events::UiEvent, utils::render_tee_for_ui_with_skin};

use super::user_data::UserData;

pub fn render_modes(ui: &mut egui::Ui, pipe: &mut UiRenderPipe<UserData>) {
    let log = &pipe.user_data.log;
    let mode = log.mode();
    if let Some(mode) = mode {
        match mode {
            ConnectModes::Connecting { addr } => {
                ui.vertical(|ui| {
                    ui.label(format!("Connecting to:\n{addr}"));
                    if ui.button("Cancel").clicked() {
                        pipe.user_data.events.push(UiEvent::Disconnect);
                        pipe.user_data.config.engine.ui.path.route("");
                    }
                });
            }
            ConnectModes::ConnectingErr { msg } => {
                ui.vertical(|ui| {
                    ui.label(format!(
                        "Connecting to {} failed:\n{msg}",
                        pipe.user_data.config.storage::<String>("server-addr")
                    ));
                    if ui.button("Return").clicked() {
                        pipe.user_data.events.push(UiEvent::Disconnect);
                        pipe.user_data.config.engine.ui.path.route("");
                    }
                });
            }
            ConnectModes::Queue { msg } => {
                ui.vertical(|ui| {
                    ui.label(format!(
                        "Connecting to {}",
                        pipe.user_data.config.storage::<String>("server-addr")
                    ));
                    ui.label(format!("Waiting in queue: {msg}"));
                    if ui.button("Cancel").clicked() {
                        pipe.user_data.events.push(UiEvent::Disconnect);
                        pipe.user_data.config.engine.ui.path.route("");
                    }
                });
            }
            ConnectModes::DisconnectErr { msg } => {
                ui.vertical(|ui| {
                    ui.label(format!(
                        "Connection to {} lost:\n{msg}",
                        pipe.user_data.config.storage::<String>("server-addr")
                    ));
                    if ui.button("Return").clicked() {
                        pipe.user_data.events.push(UiEvent::Disconnect);
                        pipe.user_data.config.engine.ui.path.route("");
                    }
                });
            }
        }
    }
}

/// top bar
/// big square, rounded edges
#[instrument(level = "trace", skip_all)]
pub fn render(ui: &mut egui::Ui, ui_state: &mut UiState, pipe: &mut UiRenderPipe<UserData>) {
    let mut mesh = Mesh::default();
    let screen = ui.ctx().screen_rect();
    mesh.colored_vertex(screen.left_top(), Color32::from_black_alpha(0));
    mesh.colored_vertex(screen.right_top(), Color32::from_black_alpha(0));
    mesh.colored_vertex(screen.right_bottom(), Color32::from_black_alpha(255));
    mesh.colored_vertex(screen.left_bottom(), Color32::from_black_alpha(255));

    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);

    ui.painter().add(mesh);

    StripBuilder::new(ui)
        .size(Size::remainder())
        .size(Size::remainder())
        .horizontal(|mut strip| {
            strip.cell(|ui| {
                let style = ui.style_mut();
                style.spacing.item_spacing.x = 25.0;
                style.visuals.widgets.hovered.corner_radius = 18.into();
                style.visuals.widgets.hovered.bg_fill = Color32::WHITE;
                style.visuals.widgets.hovered.weak_bg_fill = Color32::WHITE;
                style.visuals.widgets.hovered.bg_stroke =
                    Stroke::new(2.0, Color32::from_black_alpha(74));
                style.visuals.widgets.hovered.expansion = 10.0;
                style.visuals.widgets.hovered.fg_stroke = Stroke::new(2.0, Color32::BLACK);
                style.visuals.widgets.active.corner_radius = 18.into();
                style.visuals.widgets.active.bg_fill = Color32::from_black_alpha(41);
                style.visuals.widgets.active.weak_bg_fill = Color32::from_black_alpha(41);
                style.visuals.widgets.active.bg_stroke =
                    Stroke::new(2.0, Color32::from_black_alpha(74));
                style.visuals.widgets.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
                style.visuals.widgets.active.expansion = 8.0;
                style.visuals.widgets.inactive.corner_radius = 18.into();
                style.visuals.widgets.inactive.bg_fill = Color32::from_black_alpha(41);
                style.visuals.widgets.inactive.weak_bg_fill = Color32::from_black_alpha(41);
                style.visuals.widgets.inactive.bg_stroke =
                    Stroke::new(2.0, Color32::from_black_alpha(74));
                style.visuals.widgets.inactive.fg_stroke = Stroke::new(2.0, Color32::WHITE);
                style.visuals.widgets.inactive.expansion = 8.0;
                add_margins(ui, |ui| {
                    ui.with_layout(Layout::bottom_up(egui::Align::Min), |ui| {
                        let style = ui.style_mut();
                        style.spacing.item_spacing.x = 0.0;
                        Grid::new("cancel-tee-joining...")
                            .num_columns(4)
                            .spacing(egui::vec2(45.0, 0.0))
                            .show(ui, |ui| {
                                let label = ui.button("Cancel  \u{f00d}");
                                let mut anim_state = AnimState::default();
                                anim_state.set(&base_anim(), &Duration::from_millis(0));
                                anim_state.add(
                                    &run_right_anim(),
                                    &pipe.cur_time.saturating_mul(2),
                                    1.0,
                                );
                                let skin = pipe
                                    .user_data
                                    .skin_container
                                    .get_or_default_opt::<ResourceKey>(None);

                                let pos = label.rect.right_center();
                                let size = 20.0;
                                render_tee_for_ui_with_skin(
                                    pipe.user_data.canvas_handle,
                                    skin.clone(),
                                    pipe.user_data.tee_render,
                                    ui,
                                    ui_state,
                                    screen,
                                    None,
                                    Some(&NetworkSkinInfo::Original),
                                    vec2::new(pos.x + size / 2.0 + 20.0, pos.y),
                                    size,
                                    TeeEye::Happy,
                                    Some(anim_state),
                                );
                                ui.label(
                                    RichText::new("Joining...")
                                        .color(Color32::from_rgba_unmultiplied(255, 255, 140, 255)),
                                );
                                ui.end_row();
                            });
                    });
                });
            });
            strip.cell(|ui| {
                let style = ui.style_mut();
                style.spacing.item_spacing.x = 25.0;

                add_margins(ui, |ui| {
                    ui.with_layout(Layout::bottom_up(egui::Align::Max), |ui| {
                        let log = &pipe.user_data.log;
                        let state = log.state();
                        match state {
                            ConnectingState::DownloadingMap {
                                map_name,
                                downloaded_bytes,
                                total_download_bytes,
                                download_speed_bytes_per_second,
                            } => {
                                ui.style_mut().visuals.selection.stroke =
                                    Stroke::new(1.0, Color32::BLACK);
                                ui.add(
                                    ProgressBar::new(
                                        downloaded_bytes as f32
                                            / total_download_bytes.max(1) as f32,
                                    )
                                    .fill(Color32::WHITE)
                                    .show_percentage(),
                                );

                                /// Estimate remaining download time.
                                ///
                                /// - `downloaded_bytes`: how many bytes have been downloaded so far
                                /// - `total_download_bytes`: total size in bytes (the final expected size)
                                /// - `download_speed_bytes_per_second`: current speed in bytes/sec
                                ///
                                /// Returns `None` if we can't compute a sane estimate (e.g. speed is 0).
                                pub fn estimate_time_remaining(
                                    downloaded_bytes: u64,
                                    total_download_bytes: u64,
                                    download_speed_bytes_per_second: u64,
                                ) -> Option<Duration> {
                                    if download_speed_bytes_per_second == 0 {
                                        return None; // can't estimate without speed
                                    }

                                    if downloaded_bytes >= total_download_bytes {
                                        // already done (or somehow over)
                                        return Some(Duration::from_secs(0));
                                    }

                                    let remaining_bytes = total_download_bytes - downloaded_bytes;

                                    // Avoid intermediate overflow by doing the division first.
                                    // remaining_seconds = remaining_bytes / bytes_per_second
                                    let remaining_seconds =
                                        remaining_bytes / download_speed_bytes_per_second;

                                    // If remaining_bytes is not evenly divisible by speed, there is still a
                                    // fractional second left — round up so we don't under-estimate.
                                    let has_remainder = !remaining_bytes
                                        .is_multiple_of(download_speed_bytes_per_second);
                                    let remaining_seconds_rounded_up = if has_remainder {
                                        remaining_seconds.saturating_add(1)
                                    } else {
                                        remaining_seconds
                                    };

                                    Some(Duration::from_secs(remaining_seconds_rounded_up))
                                }
                                ui.label(
                                    RichText::new(format!(
                                        "{}/{} KiB ({} KiB/s) - {} seconds left",
                                        downloaded_bytes / 1024,
                                        total_download_bytes / 1024,
                                        download_speed_bytes_per_second / 1024,
                                        estimate_time_remaining(
                                            downloaded_bytes,
                                            total_download_bytes,
                                            download_speed_bytes_per_second
                                        )
                                        .unwrap_or_default()
                                        .as_secs()
                                    ))
                                    .color(Color32::WHITE),
                                );

                                ui.label(
                                    RichText::new(format!("Downloading map: {map_name}"))
                                        .color(Color32::WHITE),
                                );
                            }
                            ConnectingState::Other(msg) => {
                                ui.label(msg);
                            }
                            ConnectingState::None => {
                                // ignore
                            }
                        }
                    });
                });
            });
        });
}
