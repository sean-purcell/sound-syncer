use std::collections::HashMap;
use std::process::Stdio;

use futures::stream::{Stream, TryStream, TryStreamExt};

use clap::Parser;
use eyre::{eyre, Report, Result, WrapErr};
use rss::Channel;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::process::Command;

#[derive(Serialize, Deserialize, Debug)]
struct Playlist {
    name: String,
    url: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Podcast {
    name: String,
    url: String,
    keep_latest: usize,
    playback_speed: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct PodcastSet {
    name: String,
    podcasts: Vec<Podcast>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    playlists: Vec<Playlist>,
    podcasts: Vec<PodcastSet>,
}

#[derive(Parser, Debug)]
#[command()]
struct Args {
    #[arg(short, long)]
    config: String,
    #[arg(short, long)]
    output_dir: String,
}

async fn create_and_get_dir(output_dir: &str, name: &str) -> Result<String> {
    let dir = format!("{output_dir}/{name}");
    fs::create_dir_all(&dir)
        .await
        .wrap_err_with(|| format!("Failed to create directory {dir}"))?;
    Ok(dir)
}

async fn sync_playlist(playlist: &Playlist, output_dir: &str) -> Result<()> {
    let playlist_dir = create_and_get_dir(output_dir, &playlist.name).await?;
    let status = Command::new("spotdl")
        .current_dir(&playlist_dir)
        .stdin(Stdio::null())
        .arg("sync")
        .arg(&playlist.url)
        .arg("--save-file")
        .arg("playlist.spotdl")
        .status()
        .await
        .wrap_err("Failed to execute spotdl")?;
    if status.success() {
        println!("Synced playlist");
        Ok(())
    } else {
        Err(eyre!(
            "Failed to sync playlist {0}: {status:?}",
            playlist.name
        ))
    }
}

async fn sync_podcasts(podcasts: &PodcastSet, output_dir: &str) -> Result<()> {
    let podcast_dir = create_and_get_dir(output_dir, &podcasts.name).await?;
    println!("Processing podcast set {0}", podcasts.name);
    let existing_files: HashMap<_, _> = fs::read_dir(podcast_dir)
        .await
        .wrap_err("Failed to read podcast dir")?
        .map_err(Report::new)
        .try_filter_map(|entry| {
            entry
                .file_name()
                .into_string()
                .wrap_err("Failed to convert path to string")?
        })
        .collect();
    for podcast in &podcasts.podcasts {
        println!("Fetching {0}", podcast.name);
        let content = reqwest::get(&podcast.url)
            .await
            .wrap_err_with(|| {
                format!(
                    "Failed to fetch podcast {0} from {1}",
                    podcast.name, podcast.url
                )
            })?
            .bytes()
            .await?;
        let channel = Channel::read_from(&content[..])?;
        for item in channel.items.iter().take(podcast.keep_latest) {}
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    println!("Loading config: {0}", args.config);

    let config: Config = {
        let contents = std::fs::read_to_string(args.config).expect("Failed to read config file");
        serde_json::from_str(contents.as_str()).expect("Failed to parse config")
    };

    println!("Loaded config: {config:?}");

    for playlist in &config.playlists {
        sync_playlist(playlist, args.output_dir.as_str()).await?;
    }
    for podcast_set in &config.podcasts {
        sync_podcasts(&podcast_set, args.output_dir.as_str()).await?;
    }
    Ok(())
}
