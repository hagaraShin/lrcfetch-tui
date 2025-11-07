use std::{error::Error, path::PathBuf};

use ratatui::widgets::Row;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct MusicData {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: usize,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum Lyrics {
    None,
    Synced(String),
    Plain(String),
    Instrumental,
}

impl Lyrics {
    pub async fn to_file(&self, path: &PathBuf) -> Result<(), tokio::io::Error> {
        let mut path = path.clone();
        match self {
            Lyrics::None => Ok(()),
            Lyrics::Synced(lrc) => {
                path.set_extension("lrc");
                Ok(tokio::fs::write(path, lrc).await?)
            }
            Lyrics::Plain(lrc) => {
                path.set_extension("txt");
                Ok(tokio::fs::write(path, lrc).await?)
            }
            Lyrics::Instrumental => Ok(()),
        }
    }
}

impl<'a> MusicData {
    pub fn to_row(&'a self) -> Row<'a> {
        Row::new(vec![
            self.title.to_string(),
            self.artist.to_string(),
            self.album.to_string(),
        ])
    }

    pub async fn query(&self, client: &reqwest::Client) -> Lyrics {
        let response = client
            .get("https://lrclib.net/api/get")
            .query(&[
                ["track_name", self.title.as_str()],
                ["artist_name", self.artist.as_str()],
                ["album_name", self.album.as_str()],
                ["duration", self.duration.to_string().as_str()],
            ])
            .send()
            .await;
        match response {
            Ok(response) => {
                if !response.status().is_success() {
                    return Lyrics::None;
                }
                let Ok(lyrics) = response.text().await else {
                    return Lyrics::None;
                };
                match serde_json::from_str::<ApiResponse>(lyrics.as_str()) {
                    Ok(lyrics_data) => {
                        if let Some(lrc) = lyrics_data.synced_lyrics {
                            Lyrics::Synced(lrc)
                        } else if let Some(lrc) = lyrics_data.plain_lyrics {
                            Lyrics::Plain(lrc)
                        } else if lyrics_data.instrumental {
                            Lyrics::Instrumental
                        } else {
                            Lyrics::None
                        }
                    }
                    Err(err) => Lyrics::None,
                }
            }
            Err(err) => Lyrics::None,
        }
    }

    pub async fn check_lyrics(&self) -> Result<Lyrics, tokio::io::Error> {
        if let Ok(true) = self.path.with_extension("lrc").try_exists() {
            let path = self.path.with_extension("lrc");
            let lyrics = tokio::fs::read_to_string(path).await?;

            Ok(Lyrics::Synced(lyrics))
        } else if let Ok(true) = self.path.with_extension("txt").try_exists() {
            let path = self.path.with_extension("txt");
            let lyrics = tokio::fs::read_to_string(path).await?;

            Ok(Lyrics::Plain(lyrics))
        } else {
            Ok(Lyrics::None)
        }
    }

    pub fn from_file(flac_file: PathBuf) -> Result<MusicData, Box<dyn Error>> {
        let tags = metaflac::Tag::read_from_path(&flac_file)?;

        let Some(mut title) = tags.get_vorbis("TITLE") else {
            return Err("No title found".into());
        };
        let Some(mut artist) = tags.get_vorbis("ARTIST") else {
            return Err("No artist found".into());
        };
        let Some(mut album) = tags.get_vorbis("ALBUM") else {
            return Err("No album found".into());
        };

        let streaminfo = tags.get_streaminfo().unwrap();
        let duration = streaminfo.total_samples as usize / streaminfo.sample_rate as usize;

        Ok(MusicData {
            title: title.next().unwrap_or_default().to_string(),
            artist: artist.next().unwrap_or_default().to_string(),
            album: album.next().unwrap_or_default().to_string(),
            duration,
            path: flac_file,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct ApiResponse {
    instrumental: bool,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}
