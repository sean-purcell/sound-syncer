use std::fs;
use std::process::{Command, Stdio};

use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct Playlist {
    name: String,
    url: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Podcast {
    url: String,
    keep_latest: u32,
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

fn sync_playlist(playlist: &Playlist, output_dir: &str) {
    let playlist_dir = format!("{output_dir}/{0}", playlist.name);
    fs::create_dir_all(&playlist_dir).expect("Failed to create directory for playlist");
    let status = Command::new("spotdl")
        .current_dir(&playlist_dir)
        .stdin(Stdio::null())
        .arg("sync")
        .arg(&playlist.url)
        .arg("--save-file")
        .arg("playlist.spotdl")
        .status()
        .expect("Failed to execute spotdl");
    if status.success() {
        println!("Synced playlist");
    } else {
        println!("Failed to sync playlist: {status:?}");
    }
}

fn sync_podcasts(podcasts: &PodcastSet, output_dir: &str) {
    let playlist_dir = format!("{output_dir}/{0}", podcasts.name);
    fs::create_dir_all(&playlist_dir).expect("Failed to create directory for podcast set");
    for podcast in &podcasts.podcasts {}
}

fn main() {
    let args = Args::parse();
    println!("Loading config: {0}", args.config);

    let config: Config = {
        let contents = std::fs::read_to_string(args.config).expect("Failed to read config file");
        serde_json::from_str(contents.as_str()).expect("Failed to parse config")
    };

    println!("Loaded config: {config:?}");

    for playlist in &config.playlists {
        sync_playlist(playlist, args.output_dir.as_str());
    }
}