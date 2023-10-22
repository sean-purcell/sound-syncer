use std::collections::{HashMap, HashSet};
use std::process::Stdio;

use futures::stream::TryStreamExt;

use chrono::{DateTime, Utc};
use clap::Parser;
use eyre::{eyre, Report, Result, WrapErr};
use filetime::{self, FileTime};
use rss::Channel;
use serde::{Deserialize, Serialize};
use tokio::{fs, process::Command};
use tokio_stream::wrappers::ReadDirStream;

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
    playback_speed: f64,
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

fn fat32_sanitize(f: &str) -> String {
    f.chars()
        .filter(|c| match c {
            '0'..='9' | 'a'..='z' | 'A'..='Z' | ' ' | '-' | '_' | '.' => true,
            _ => false,
        })
        .collect()
}

fn podcast_file_name(podcast: &str, title: &str) -> String {
    format!(
        "{} - {}.mp3",
        fat32_sanitize(podcast),
        fat32_sanitize(title)
    )
}

async fn sync_podcasts(podcasts: &PodcastSet, output_dir: &str) -> Result<()> {
    let podcast_dir = create_and_get_dir(output_dir, &podcasts.name).await?;
    println!("Processing podcast set {0}", podcasts.name);
    let read_dir = fs::read_dir(&podcast_dir)
        .await
        .wrap_err("Failed to read podcast dir")?;
    let existing_files: HashMap<String, DateTime<Utc>> = ReadDirStream::new(read_dir)
        .map_err(Report::new)
        .try_filter_map(|entry| async move {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|name| eyre!("Failed to convert file name to string {name:?}"))?;
            let metadata = entry
                .metadata()
                .await
                .wrap_err("Failed to get metadata for file")?;
            if metadata.is_file() {
                let mtime = DateTime::from_timestamp(
                    FileTime::from_last_modification_time(&metadata).unix_seconds(),
                    0,
                )
                .unwrap();
                Ok(Some((name, mtime)))
            } else {
                Ok(None)
            }
        })
        .try_collect()
        .await?;

    println!("Existing files:");
    existing_files.iter().for_each(|(name, size)| {
        println!("- {name}: {size} bytes");
    });
    let mut expected_files = HashSet::<String>::new();

    let tempdir = tempfile::tempdir().wrap_err("Failed to create temp dir")?;

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
        for item in channel.items.iter().take(podcast.keep_latest) {
            let title = item
                .title
                .clone()
                .ok_or_else(|| eyre!("Podcast {0} item doesn't have title", podcast.name))?;
            let pub_date_str = item.pub_date.clone().ok_or_else(|| {
                eyre!(
                    "Podcast {0} episode {title} doesn't have pub date",
                    podcast.name
                )
            })?;
            let pub_time = DateTime::parse_from_rfc2822(pub_date_str.as_str())
                .wrap_err_with(|| format!("Failed to parse date time {pub_date_str}"))?;
            let enclosure = item.enclosure.clone().ok_or_else(|| {
                eyre!(
                    "Podcast {0} episode {title} doesn't have enclosure",
                    podcast.name
                )
            })?;
            let file_name = podcast_file_name(podcast.name.as_str(), title.as_str());
            let redownload = match existing_files.get(&file_name) {
                Some(mtime) => {
                    if mtime.timestamp() == pub_time.timestamp() {
                        println!(
                            "Not redownloading {file_name} as it has the expected mtime ({pub_time})"
                        );
                        false
                    } else {
                        println!("Redownloading {file_name} as it has the wrong mtime ({mtime}, expected {pub_time})");
                        true
                    }
                }
                None => {
                    println!("{file_name} not present, downloading");
                    true
                }
            };

            if redownload {
                let url = enclosure.url;
                println!("Downloading {file_name} from {url}");
                let temp_file = tempdir.path().join(&file_name);
                tokio::process::Command::new("curl")
                    .arg("-L")
                    .arg(url)
                    .arg("-o")
                    .arg(&temp_file)
                    .stdin(Stdio::null())
                    .status()
                    .await
                    .wrap_err("Failed to download episode")?;
                println!("Adjusting playback speed");
                let path = format!("{podcast_dir}/{file_name}");
                tokio::process::Command::new("ffmpeg")
                    .arg("-i")
                    .arg(&temp_file)
                    .arg("-filter:a")
                    .arg(format!("atempo={0}", podcast.playback_speed))
                    .arg(&path)
                    .stdin(Stdio::null())
                    .status()
                    .await
                    .wrap_err("Failed to download episode")?;
                filetime::set_file_mtime(&path, FileTime::from_unix_time(pub_time.timestamp(), 0))
                    .wrap_err("Failed to set mtime")?;
            }
            expected_files.insert(file_name.clone());
        }
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
