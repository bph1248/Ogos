use ogos_config as config;
use config::*;
use ogos_core::*;
use ogos_err::*;

use discord_rich_presence::{
    *,
    activity::*
};
use log::*;
use std::time;
use strum::*;

#[derive(Display)]
pub enum Msg {
    Close
}

pub fn begin(ipc_client: &mut DiscordIpcClient, info: &DiscordActivityInfoView, display_kind: DiscordDisplayKind) -> Res<()> {
    info!("{}: begin", module_path!());

    let time_start = i64::try_from(time::SystemTime::now().duration_since(time::UNIX_EPOCH)?.as_secs())?;
    let mut activity = Activity::new()
        .activity_type(info.activity.into())
        .timestamps(Timestamps::new().start(time_start))
        .details(info.details);
    activity = match info.state {
        Some(state) => activity.state(state).status_display_type(display_kind.into()),
        None if display_kind == DiscordDisplayKind::State => activity.status_display_type(DiscordDisplayKind::Details.into()), // Fallback
        _ => activity.status_display_type(display_kind.into())
    };
    if let Some(large_image) = info.large_image {
        activity = activity.assets(Assets::new().large_image(large_image));
    }

    ipc_client.connect()?;
    ipc_client.set_activity(activity)?;

    info!("{}: set discord activity: id: {}, details: {}, state: {}, large_image: {}", module_path!(), ipc_client.client_id, info.details, info.state.as_display(), info.large_image.as_display());

    Ok(())
}
