use crate::{
    common::*,
    config::*
};
use ogos_err::*;

use concat_string::*;
use discord_rich_presence::{
    *,
    activity::*
};
use log::*;
use serde::*;
use std::{
    os::windows::process::*,
    process::*,
    sync::mpsc::*,
    thread::*,
    time::{self, *}
};

#[allow(dead_code)]
#[derive(Deserialize)]
struct Last {
    rating: i32,
    date: u64,
    rd: u32
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Best {
    rating: u32,
    date: u64,
    game: String
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Record {
    win: u32,
    loss: u32,
    draw: u32
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ChessRapid {
    last: Last,
    best: Best,
    record: Record
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Extreme {
    rating: u32,
    date: u64
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Tactics {
    highest: Extreme,
    lowest: Extreme
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Stats {
    chess_rapid: ChessRapid,
    fide: u32,
    tactics: Tactics
}

pub(crate) fn begin(ipc_client: &mut DiscordIpcClient, info: &DiscordInfo) -> Res<()> {
    info!("{}: begin", module_path!());

    ipc_client.connect()?;

    let time_start = i64::try_from(time::SystemTime::now().duration_since(time::UNIX_EPOCH)?.as_secs())?;
    let mut activity = Activity::new()
        .activity_type(info.activity.into())
        .timestamps(Timestamps::new().start(time_start))
        .details(info.details.as_str())
        .status_display_type(info.display_kind.into());

    if let Some(state) = info.state.as_ref() {
        activity = activity.state(state);
    }
    if let Some(large_image) = info.large_image.as_ref() {
        activity = activity.assets(Assets::new().large_image(large_image));
    }

    ipc_client.set_activity(activity)?;

    info!("{}: set discord activity: id: {}, details: {}, state: {:?}, large_image: {:?}", module_path!(), ipc_client.client_id, info.details, info.state, info.large_image);

    Ok(())
}

fn begin_chess(ipc_client: &mut DiscordIpcClient, large_image: Option<String>, username: String, rx: Receiver<Msg>) -> Res<()> {
    info!("{}: begin chess", module_path!());

    ipc_client.connect()?;

    let url = concat_string!("https://api.chess.com/pub/player/", username.to_lowercase(), "/stats");

    let mut cmd = Command::new("curl");
    cmd.arg(url).creation_flags(CREATE_NO_WINDOW);
    let init = || -> Res<(Stats, i64)> {
        let output = output_command(&mut cmd)?.stdout;
        let initial_stats = serde_json::from_slice::<Stats>(output.as_slice())?;

        let time_start = i64::try_from(time::SystemTime::now().duration_since(time::UNIX_EPOCH)?.as_secs())?;
        let details = format!(
            "Chess.com Rapid ELO: {} ({:+})",
            initial_stats.chess_rapid.last.rating,
            0
        );
        let state = format!(
            "W/L: 0-0 ({}-{})",
            initial_stats.chess_rapid.record.win,
            initial_stats.chess_rapid.record.loss
        );

        let mut activity = Activity::new()
            .activity_type(ActivityType::Playing)
            .timestamps(Timestamps::new().start(time_start))
            .details(details.as_str())
            .state(state.as_str());

        if let Some(large_image) = large_image.as_ref() {
            activity = activity.assets(Assets::new().large_image(large_image));
        }

        ipc_client.set_activity(activity)?;

        info!("{}: set discord activity: chess username: {}, id: {}, details: {}, state: {}, large_image: {:?}", module_path!(), username, ipc_client.client_id, details, state, large_image);

        Ok((initial_stats, time_start))
    };

    let (initial_stats, time_start) = attempt(init, 3, Duration::from_secs(3))
        .inspect_err(|err| {
            error!("{}: failed to init chess stats: {}", module_path!(), err);
        })?;

    loop {
        match rx.recv_timeout(Duration::from_secs(30)) { // Wait for signal to close, triggered when user closes gui
            Ok(msg) => {
                if let Msg::Close = msg {
                    break
                }
            },
            Err(err) => {
                if let RecvTimeoutError::Disconnected = err {
                    Err(err)?;
                }
            }
        }

        let mut update = || -> Res<()> {
            let output = output_command(&mut cmd)?.stdout;
            let stats = serde_json::from_slice::<Stats>(output.as_slice())?;

            let details = format!(
                "Chess.com Rapid ELO: {} ({:+})",
                stats.chess_rapid.last.rating,
                stats.chess_rapid.last.rating - initial_stats.chess_rapid.last.rating
            );
            let state = format!(
                "W/L: {}-{} ({}-{})",
                stats.chess_rapid.record.win - initial_stats.chess_rapid.record.win,
                stats.chess_rapid.record.loss - initial_stats.chess_rapid.record.loss,
                stats.chess_rapid.record.win,
                stats.chess_rapid.record.loss
            );

            let mut activity = Activity::new()
                .activity_type(ActivityType::Playing)
                .timestamps(Timestamps::new().start(time_start))
                .details(details.as_str())
                .state(state.as_str());

            if let Some(large_image) = large_image.as_ref() {
                activity = activity.assets(Assets::new().large_image(large_image));
            }

            ipc_client.set_activity(activity)?;

            Ok(())
        };

        if let Err(err) = update() {
            warn!("{}: failed to update chess stats: {}", module_path!(), err);

            ipc_client.reconnect()?;
        }
    }

    Ok(())
}

pub(crate) fn spawn_scoped_chess<'a>(s: &'a Scope<'a, '_>, ipc_client: &'a mut DiscordIpcClient, large_image: Option<String>, username: String, rx: Receiver<Msg>) -> ScopedJoinHandle<'a, ()> {
    s.spawn(|| {
        begin_chess(ipc_client, large_image, username, rx).unwrap_or_else(|err| {
            error!("{}: failed to monitor chess stats: {}", module_path!(), err);
        });
    })
}
