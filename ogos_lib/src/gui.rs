use crate::{
    common::*,
    config::{self, *},
    discord,
    err::*,
    video
};

use discord_rich_presence::*;
use eframe::{
    egui,
    egui_wgpu,
    wgpu
};
use log::*;
use serde::*;
use std::{
    fs::*,
    path::*,
    thread
};

struct DirEntryInfo {
    file_kind: FileKind,
    file_stem: String,
    path: PathBuf
}

pub(crate) enum Kind {
    Discord { name: String, rp_info: DiscordRichPresenceInfo },
    Dir { name: String, entries: Vec<DirEntry>, discord_app_ids: DiscordAppIds }
}

#[derive(Default, Deserialize, PartialEq)]
enum Watching {
    Movie,
    #[default]
    TV,
    Words
}

fn to_discord_rp_asset_name(s: impl AsRef<str>) -> String {
    s.as_ref().chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();

            match c {
                '\'' | '.' | ' ' => '_',
                _ => c
            }
        })
        .collect()
}

pub(crate) struct Discord {
    name: String,
    rp_info: DiscordRichPresenceInfo
}
impl Discord {
    pub(crate) fn new(_cctx: &eframe::CreationContext<'_>, name: String, rp_info: DiscordRichPresenceInfo) -> Self {
        Self {
            name,
            rp_info
        }
    }
}
impl eframe::App for Discord {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::new([false, true]).show(ui, |ui| {
                ui.heading(&self.name);

                ui.separator();

                let text_edit = egui::TextEdit::singleline(&mut self.rp_info.details).desired_width(f32::INFINITY);
                let details = ui.label("Details");

                ui.add(text_edit).labelled_by(details.id);
            });
        });
    }
}

#[derive(Default)]
pub(crate) struct Dir {
    name: String,
    valid_entries: Vec<DirEntryInfo>,
    entry_errored: bool,
    hovered_valid_entry_index: usize,
    maintain_sample_rate: bool,
    use_glsl_shaders: bool,
    discord_app_ids: config::DiscordAppIds,
    discord_rp_enable: bool,
    discord_rp_watching: Watching,
    discord_rp_details: String,
    discord_rp_state: String
}
impl Dir {
    pub(crate) fn new(_cctx: &eframe::CreationContext<'_>, name: String, entries: Vec<DirEntry>, discord_app_ids: DiscordAppIds) -> Self {
        let valid_entries = entries.iter()
            .filter_map(|entry| {
                let is_file = entry.file_type()
                    .map(|file_type| {
                        file_type.is_file()
                    })
                    .unwrap_or(false);

                match is_file {
                    true => {
                        let path = entry.path();

                        path.get_extension().ok()
                            .zip(path.get_file_stem().ok())
                            .map(|(file_ext, file_stem)| {
                                DirEntryInfo {
                                    file_kind: get_file_kind(file_ext),
                                    file_stem: file_stem.replace("¿", "?"),
                                    path: entry.path()
                                }
                            })
                    },
                    false => None
                }
            })
            .collect();

        Self {
            name,
            valid_entries,
            discord_app_ids,
            ..default!()
        }
    }
}
impl eframe::App for Dir {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Options", |ui| {
                    ui.menu_button("Discord Rich Presence", |ui| {
                        ui.horizontal(|ui| {
                            let can_enable_discord_rp = match self.discord_rp_watching {
                                Watching::Movie => self.discord_app_ids.movies.is_some(),
                                Watching::TV => self.discord_app_ids.tv.is_some(),
                                Watching::Words => self.discord_app_ids.words.is_some()
                            };
                            self.discord_rp_enable &= can_enable_discord_rp;

                            ui.add_enabled(can_enable_discord_rp, egui::Checkbox::new(&mut self.discord_rp_enable, "Enable"));

                            ui.separator();

                            if ui.add_enabled(can_enable_discord_rp, egui::RadioButton::new(self.discord_rp_watching == Watching::TV, "TV")).clicked() {
                                self.discord_rp_watching = Watching::TV;
                            };
                            if ui.add_enabled(can_enable_discord_rp, egui::RadioButton::new(self.discord_rp_watching == Watching::Movie, "Movie")).clicked() {
                                self.discord_rp_watching = Watching::Movie;
                            };
                            if ui.add_enabled(can_enable_discord_rp, egui::RadioButton::new(self.discord_rp_watching == Watching::Words, "Words")).clicked() {
                                self.discord_rp_watching = Watching::Words;
                            };
                        });

                        ui.shrink_width_to_current();

                        let details_label_galley = egui::WidgetText::from("Details")
                            .into_galley(ui, None, f32::INFINITY, egui::FontSelection::Default);
                        let details_text_edit_width = ui.available_width() - details_label_galley.rect.width();

                        let margin = egui::Margin::symmetric(4.0, 2.0);
                        let grid = egui::Grid::new("grid").num_columns(2);
                        grid.show(ui, |ui| {
                            ui.label(details_label_galley);

                            let details_hint_text = match self.discord_rp_watching {
                                Watching::TV => self.name.as_str(),
                                _ => self.valid_entries[self.hovered_valid_entry_index].file_stem.as_str()
                            };
                            let details_hint_galley = egui::WidgetText::from(details_hint_text)
                                .into_galley(ui, Some(egui::TextWrapMode::Truncate), ui.available_width() - margin.sum().x, egui::FontSelection::Default);

                            let details_text_edit = egui::TextEdit::singleline(&mut self.discord_rp_details)
                                .desired_width(details_text_edit_width)
                                .hint_text(details_hint_galley);

                            ui.add(details_text_edit);

                            ui.end_row();

                            if self.discord_rp_watching == Watching::TV {
                                ui.label("State");

                                let state_hint_text = self.valid_entries[self.hovered_valid_entry_index].file_stem.as_str();
                                let state_hint_galley = egui::WidgetText::from(state_hint_text)
                                    .into_galley(ui, Some(egui::TextWrapMode::Truncate), ui.available_width() - margin.sum().x, egui::FontSelection::Default);
                                let state_text_edit = egui::TextEdit::singleline(&mut self.discord_rp_state)
                                    .hint_text(state_hint_galley)
                                    .desired_width(details_text_edit_width);

                                ui.add(state_text_edit);
                            }
                        });
                    });

                    ui.checkbox(&mut self.maintain_sample_rate, "Maintain sample rate");
                    ui.checkbox(&mut self.use_glsl_shaders, "Use glsl shaders");
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::new([false, true]).show(ui, |ui| {
                ui.heading(&self.name);

                ui.separator();

                self.valid_entries.iter()
                    .enumerate()
                    .try_for_each(|(i, valid_entry_info)| -> Res<()> {
                        let DirEntryInfo { file_kind, file_stem, path} = valid_entry_info;

                        let button = ui.button(file_stem);

                        if button.clicked() {
                            let mut discord_rp_info = match self.discord_rp_enable {
                                true => {
                                    let discord_rp_info = config::DiscordRichPresenceInfo {
                                        client_id: match self.discord_rp_watching {
                                            Watching::Movie => self.discord_app_ids.movies.clone().unwrap(),
                                            Watching::TV => self.discord_app_ids.tv.clone().unwrap(),
                                            Watching::Words => self.discord_app_ids.words.clone().unwrap()
                                        },
                                        activity: config::DiscordActivity::Watching,
                                        details: match self.discord_rp_details.is_empty() {
                                            true => match self.discord_rp_watching {
                                                Watching::TV => self.name.clone(),
                                                _ => file_stem.into()
                                            },
                                            false => self.discord_rp_details.clone()
                                        },
                                        state: match self.discord_rp_watching == Watching::TV {
                                            true => {
                                                match self.discord_rp_state.is_empty() {
                                                    true => Some(file_stem.into()),
                                                    false => Some(self.discord_rp_state.clone())
                                                }
                                            },
                                            false => None
                                        },
                                        large_image: {
                                            match self.discord_rp_watching {
                                                Watching::TV => Some(to_discord_rp_asset_name(&self.name)),
                                                _ => Some(to_discord_rp_asset_name(file_stem))
                                            }
                                        },
                                        chess_username: None
                                    };

                                    Some(discord_rp_info)
                                },
                                false => None
                            };

                            let path = path.clone();
                            let file_kind = *file_kind;
                            let maintain_sample_rate = self.maintain_sample_rate.into();
                            let use_glsl_shaders = self.use_glsl_shaders;

                            unsafe {
                                thread::Builder::new()
                                    .spawn(move || {
                                        (|| -> Res<()> {
                                            let ipc_client = match discord_rp_info.as_mut() {
                                                Some(discord_rp_info) => {
                                                    let mut ipc_client = DiscordIpcClient::new(discord_rp_info.client_id.as_ref())?;

                                                    discord::begin(&mut ipc_client, discord_rp_info)?;

                                                    Some(ipc_client)
                                                },
                                                None => None
                                            };

                                            match file_kind {
                                                FileKind::Vid => video::launch_mpv(&path, maintain_sample_rate, use_glsl_shaders)?,
                                                FileKind::Other => opener::open(&path)?
                                            }

                                            if let Some(mut ipc_client) = ipc_client {
                                                ipc_client.close()?;
                                            }

                                            Ok(())
                                        })()
                                        .unwrap_or_else(|err| {
                                            error!("{}: failed to launch media: {}", module_path!(), err);
                                        });
                                    })?;
                            }
                        }

                        if button.hovered() {
                            self.hovered_valid_entry_index = i;
                        }

                        ui.add_space(1.0);

                        Ok(())
                    })
                    .unwrap_or_else(|err| {
                        if !self.entry_errored {
                            error!("{}: failed to handle dir entry: {}", module_path!(), err);

                            self.entry_errored = true;
                        }

                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    });
            });
        });
    }
}

pub(crate) fn begin(kind: Kind) -> Res<(), { loc_var!(Gui) }> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([100.0, 100.0])
            .with_maximize_button(false)
            .with_active(true),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            supported_backends: wgpu::Backends::VULKAN,
            present_mode: wgpu::PresentMode::Mailbox,
            desired_maximum_frame_latency: Some(1),
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..default!()
        },
        follow_system_theme: true,
        ..default!()
    };

    eframe::run_native(
        "Ogos",
        native_options,
        Box::new(|cctx| {
            Ok(
                match kind {
                    Kind::Discord { name, rp_info } => Box::new(Discord::new(cctx, name, rp_info)),
                    Kind::Dir { name, entries, discord_app_ids } => Box::new(Dir::new(cctx, name, entries, discord_app_ids))
                }
            )
        })
    )?;

    Ok(())
}
