use std::{
    collections::{HashMap, VecDeque},
    env::{self, current_dir, home_dir},
    path::{PathBuf, absolute},
    sync::Arc,
    time::Duration,
    usize,
};

use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, sync::Semaphore, task::JoinSet};
mod musicdata;

use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Layout, Rect},
    style::{Color, Style},
    text::Text,
    widgets::{self, Block, Clear, List, ListState, StatefulWidget, Table, TableState, Widget},
};

use crate::musicdata::{Lyrics, MusicData};
const KEYMAP: [(KeyBind, Func); 10] = [
    (
        KeyBind {
            keycode: KeyCode::Char('a'),
            screen: Screens::Main,
        },
        Func::ScanAll,
    ),
    (
        KeyBind {
            keycode: KeyCode::Char('f'),
            screen: Screens::Main,
        },
        Func::OpenFiltersPopup,
    ),
    (
        KeyBind {
            keycode: KeyCode::Char('q'),
            screen: Screens::Filters,
        },
        Func::CloseFiltersPopup,
    ),
    (
        KeyBind {
            keycode: KeyCode::Char('j'),
            screen: Screens::Main,
        },
        Func::SelectNext,
    ),
    (
        KeyBind {
            keycode: KeyCode::Char('k'),
            screen: Screens::Main,
        },
        Func::SelectPrevious,
    ),
    (
        KeyBind {
            keycode: KeyCode::Char('j'),
            screen: Screens::Filters,
        },
        Func::FiltersSelectNext,
    ),
    (
        KeyBind {
            keycode: KeyCode::Char('k'),
            screen: Screens::Filters,
        },
        Func::FiltersSelectPrevious,
    ),
    (
        KeyBind {
            keycode: KeyCode::Enter,
            screen: Screens::Filters,
        },
        Func::OpenSelectedFilter,
    ),
    (
        KeyBind {
            keycode: KeyCode::Char('q'),
            screen: Screens::Main,
        },
        Func::Quit,
    ),
    (
        KeyBind {
            keycode: KeyCode::Enter,
            screen: Screens::Main,
        },
        Func::ScanSelected,
    ),
];

const HIGHLIGHT_STYLE: Style = Style::new().bg(Color::White).fg(Color::Black);
const MUSIC_EXTENSIONS: [&str; 1] = ["flac"];

#[derive(Default)]
struct Filter {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
}
impl Filter {
    fn apply(&self, item: &MusicData) -> bool {
        if let Some(album_filter) = &self.album {
            if !item
                .album
                .to_ascii_lowercase()
                .contains(album_filter.to_ascii_lowercase().as_str())
            {
                return false;
            }
        }
        if let Some(artist_filter) = &self.artist {
            if !item
                .artist
                .to_ascii_lowercase()
                .contains(artist_filter.to_ascii_lowercase().as_str())
            {
                return false;
            }
        }
        if let Some(title_filter) = &self.title {
            if !item
                .title
                .to_ascii_lowercase()
                .contains(title_filter.to_ascii_lowercase().as_str())
            {
                return false;
            }
        }
        true
    }
    fn to_widget(&self) -> List {
        let mut list = Vec::new();
        if let Some(title) = &self.title {
            list.push(Text::raw(format!("Title: {}", title)).centered());
        } else {
            list.push(Text::raw(format!("Title:")).centered());
        }
        if let Some(artist) = &self.artist {
            list.push(Text::raw(format!("Artist: {}", artist)).centered());
        } else {
            list.push(Text::raw(format!("Artist:")).centered());
        }
        if let Some(album) = &self.album {
            list.push(Text::raw(format!("Album: {}", album)).centered());
        } else {
            list.push(Text::raw(format!("Album:")).centered());
        }
        List::new(list)
    }
}

#[derive(Clone)]
struct Screen<'a> {
    tracks: Table<'a>,
}

impl Screen<'_> {
    fn render_text_input(&self, area: Rect, buf: &mut Buffer, state: &mut State) {
        use ratatui::layout::Constraint::{Length, Percentage};
        use ratatui::layout::Flex::Center;

        let [area] = Layout::vertical([Length(3)]).flex(Center).areas(area);
        let [area] = Layout::horizontal([Percentage(50)])
            .flex(Center)
            .areas(area);
        Clear::default().render(area, buf);
        let border = Block::bordered()
            .title("Input")
            .title_alignment(Alignment::Center);
        let inner = border.inner(area);
        border.render(area, buf);
        let text = Text::raw(state.current_string.as_str()).centered();
        text.render(inner, buf);
    }
    fn render_filters_popup(&self, area: Rect, buf: &mut Buffer, state: &mut State) {
        use ratatui::layout::Constraint::{Length, Percentage};
        use ratatui::layout::Flex::Center;

        let [area] = Layout::vertical([Length(5)]).flex(Center).areas(area);
        let [area] = Layout::horizontal([Percentage(50)])
            .flex(Center)
            .areas(area);
        Clear::default().render(area, buf);
        let border = Block::bordered()
            .title("Filters")
            .title_alignment(Alignment::Center);
        let inner = border.inner(area);
        border.render(area, buf);
        let list = state.filter.to_widget().highlight_style(HIGHLIGHT_STYLE);
        StatefulWidget::render(list, inner, buf, &mut state.filters_popup_state);
    }
}

impl Default for Screen<'_> {
    fn default() -> Self {
        return Screen {
            tracks: Table::default().row_highlight_style(HIGHLIGHT_STYLE),
        };
    }
}

impl StatefulWidget for Screen<'_> {
    type State = State;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use ratatui::layout::Constraint::{Fill, Length, Min};

        let vertical = Layout::vertical([Length(1), Min(0), Length(1), Length(1)]);
        let [title_area, main_area, progress_area, status_area] = vertical.areas(area);
        let horizontal = Layout::horizontal([Fill(1); 2]);
        let [left_area, right_area] = horizontal.areas(main_area);
        let block = Block::bordered().title("Tracks");
        StatefulWidget::render(
            self.tracks.clone(),
            block.inner(left_area),
            buf,
            &mut state.table_state,
        );
        block.render(left_area, buf);
        let progress_bar = widgets::Gauge::default().ratio(if state.total == 0 {
            1.0
        } else {
            state.done as f64 / state.total as f64
        });
        progress_bar.render(progress_area, buf);
        let txt = Text::raw("LRC Fetch").alignment(Alignment::Center);
        txt.render(title_area, buf);
        let txt = Text::raw("q - quit, j - down, k - up").alignment(Alignment::Center);
        txt.render(status_area, buf);
        let block = Block::bordered().title("Lyrics");
        'lyrics: {
            let Some(selected) = state.table_state.selected() else {
                break 'lyrics;
            };
            let Some(item) = state
                .music
                .iter()
                .filter(|x| state.filter.apply(x))
                .nth(selected)
            else {
                break 'lyrics;
            };
            if let Some(lyric) = state.lyrics.get(&item.path) {
                let txt = match lyric {
                    Lyrics::None => Text::raw("None"),
                    Lyrics::Instrumental => Text::raw("Instrumental"),
                    Lyrics::Plain(txt) => Text::raw(txt),
                    Lyrics::Synced(txt) => Text::raw(txt),
                };
                txt.render(block.inner(right_area), buf);
            } else {
                let txt = Text::raw("Not found");
                txt.render(block.inner(right_area), buf);
            }
        }
        block.render(right_area, buf);
        if let Some(_) = &state.field {
            self.render_text_input(area, buf, state);
        } else if state.screen == Screens::Filters {
            self.render_filters_popup(area, buf, state);
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Settings {
    #[serde(default = "default_concurrent")]
    concurrent_queries: usize,
    #[serde(default = "default_music_path")]
    music_path: PathBuf,
}

fn default_concurrent() -> usize {
    50
}

fn default_music_path() -> PathBuf {
    if let Ok(Ok(path)) = std::env::var("XDG_MUSIC_DIR").map(|path| absolute(path)) {
        path
    } else if let Ok(Ok(path)) =
        std::env::var("HOME").map(|path| absolute(path).map(|path| path.join("Music")))
    {
        path
    } else if let Ok(path) = current_dir() {
        path
    } else {
        PathBuf::from(".")
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            concurrent_queries: 50,
            music_path: default_music_path(),
        }
    }
}

struct State {
    settings: Settings,
    screen: Screens,
    table_state: TableState,
    will_quit: bool,
    music: Vec<MusicData>,
    lyrics: HashMap<PathBuf, Lyrics>,
    total: usize,
    done: usize,
    api_joins: tokio::task::JoinSet<LyricsRecord>,
    write_joins: tokio::task::JoinSet<Result<(), tokio::io::Error>>,
    client: reqwest::Client,
    client_limiter: Arc<Semaphore>,
    file_limiter: Arc<Semaphore>,
    filter: Filter,
    field: Option<Fields>,
    current_string: String,
    filters_popup_state: ListState,
}

#[derive(Clone)]
enum Fields {
    Title,
    Artist,
    Album,
}

impl State {
    fn event_handler(&mut self, event: Event, keymap: &HashMap<KeyBind, Func>) {
        match event {
            Event::Key(event) => {
                if !event.is_press() {
                    return;
                }
                match self.field.clone() {
                    Some(field) => match event.code {
                        KeyCode::Enter => {
                            let str = if self.current_string.is_empty() {
                                None
                            } else {
                                let res = Some(self.current_string.clone());
                                self.current_string = String::new();
                                res
                            };
                            self.set_field(field, str);
                            self.field = None;
                        }
                        KeyCode::Char(c) => {
                            self.current_string.push(c);
                        }
                        KeyCode::Backspace => {
                            self.current_string.pop();
                        }
                        _ => {}
                    },
                    None => {
                        let code = KeyBind {
                            screen: self.screen,
                            keycode: event.code,
                        };
                        if let Some(func) = keymap.get(&code) {
                            func.call(self);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn set_field(&mut self, field: Fields, value: Option<String>) {
        match field {
            Fields::Title => self.filter.title = value,
            Fields::Artist => self.filter.artist = value,
            Fields::Album => self.filter.album = value,
        }
    }
}

struct LyricsRecord {
    lyrics: Lyrics,
    path: PathBuf,
}

impl LyricsRecord {
    fn save(&self, state: &mut State) {
        let path = self.path.clone();
        let lyrics = self.lyrics.clone();
        let sema = state.file_limiter.clone();
        state.write_joins.spawn(async move {
            let lock = sema.acquire_owned().await.unwrap();
            lyrics.to_file(&path).await?;
            drop(lock);
            Ok(())
        });
    }
}

impl<'a> Default for State {
    fn default() -> Self {
        return State {
            screen: Screens::Main,
            will_quit: false,
            table_state: TableState::default().with_selected(Some(0)),
            music: Vec::default(),
            lyrics: HashMap::default(),
            total: 0,
            done: 0,
            api_joins: tokio::task::JoinSet::new(),
            write_joins: tokio::task::JoinSet::new(),
            client: reqwest::ClientBuilder::new()
                .user_agent("LRCFETCH v0.0.0 (https://github.com/hagaraShin/lrcfetch-tui)")
                .build()
                .unwrap(),
            client_limiter: Arc::new(Semaphore::new(50)),
            file_limiter: Arc::new(Semaphore::new(50)),
            settings: Settings::default(),
            filter: Filter::default(),
            field: None,
            current_string: String::new(),
            filters_popup_state: ListState::default(),
        };
    }
}

#[derive(Hash, Clone, Copy, PartialEq, Eq)]
enum Screens {
    Main,
    Filters,
}

#[derive(Hash, PartialEq, Eq)]
struct KeyBind {
    screen: Screens,
    keycode: KeyCode,
}

#[derive(Serialize, Deserialize, Debug)]
enum Func {
    ScanAll,
    ScanSelected,
    SelectNext,
    SelectPrevious,
    OpenFiltersPopup,
    OpenFilterTitle,
    OpenFilterAlbum,
    OpenFilterArtist,
    Quit,
    CloseFiltersPopup,
    FiltersSelectNext,
    FiltersSelectPrevious,
    OpenSelectedFilter,
}

fn default_config_path() -> Option<PathBuf> {
    if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
        if let Ok(mut path) = absolute(xdg_config_home) {
            path.push("lrcfetch");
            path.push("config.ron");
            if path.exists() {
                return Some(path);
            }
        };
    }
    if let Some(mut home) = home_dir() {
        home.push(".config");
        home.push("lrcfetch");
        home.push("config.ron");
        if home.exists() {
            return Some(home);
        };
    }
    if let Ok(mut pwd) = current_dir() {
        pwd.push("config.ron");
        if pwd.exists() {
            return Some(pwd);
        };
    }
    None
}

fn default_future_config_path() -> Option<PathBuf> {
    if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
        if let Ok(mut path) = absolute(xdg_config_home) {
            path.push("lrcfetch");
            path.push("config.ron");
            return Some(path);
        };
    }
    if let Some(mut home) = home_dir() {
        home.push(".config");
        home.push("lrcfetch");
        home.push("config.ron");
        return Some(home);
    }
    if let Ok(mut pwd) = current_dir() {
        pwd.push("config.ron");
        return Some(pwd);
    }
    None
}

impl Func {
    fn call(&self, state: &mut State) {
        match self {
            Func::ScanAll => Self::scan_all(state),
            Func::ScanSelected => Self::scan_song(state),
            Func::SelectNext => Self::select_next(state),
            Func::SelectPrevious => Self::select_previous(state),
            Func::Quit => Self::quit(state),
            Func::OpenFilterTitle => {
                state.current_string = state.filter.title.clone().unwrap_or(String::new());
                state.field = Some(Fields::Title);
            }
            Func::OpenFilterAlbum => {
                state.current_string = state.filter.album.clone().unwrap_or(String::new());
                state.field = Some(Fields::Album);
            }
            Func::OpenFilterArtist => {
                state.current_string = state.filter.artist.clone().unwrap_or(String::new());
                state.field = Some(Fields::Artist);
            }
            Func::OpenFiltersPopup => {
                state.screen = Screens::Filters;
            }
            Func::CloseFiltersPopup => {
                state.screen = Screens::Main;
            }
            Func::FiltersSelectNext => state.filters_popup_state.select_next(),
            Func::FiltersSelectPrevious => state.filters_popup_state.select_previous(),
            Func::OpenSelectedFilter => match state.filters_popup_state.selected() {
                Some(0) => {
                    state.current_string = state.filter.title.clone().unwrap_or(String::new());
                    state.field = Some(Fields::Title);
                }
                Some(1) => {
                    state.current_string = state.filter.artist.clone().unwrap_or(String::new());
                    state.field = Some(Fields::Artist);
                }
                Some(2) => {
                    state.current_string = state.filter.album.clone().unwrap_or(String::new());
                    state.field = Some(Fields::Album);
                }
                _ => {}
            },
        }
    }

    fn set_concurrent_queries(state: &mut State, value: usize) {
        state.client_limiter.forget_permits(usize::MAX);
        state.client_limiter.add_permits(value);
        state.settings.concurrent_queries = value;
    }
    async fn set_settings(state: &mut State, settings: Settings) {
        state.settings = settings;
        Func::set_concurrent_queries(state, state.settings.concurrent_queries);
        let Some(data) = scan_music(state.settings.music_path.clone()) else {
            return;
        };
        state.music = data;
        let mut joinset = JoinSet::new();
        for music in state.music.iter() {
            let path = music.path.clone();
            let music = music.clone();
            joinset.spawn(async move { (path, music.check_lyrics().await) });
        }
        while let Some(result) = joinset.join_next().await {
            if let Ok((path, Ok(lyrics))) = result {
                state.lyrics.insert(path, lyrics);
            }
        }
    }
    fn scan_song(state: &mut State) {
        let Some(selected) = state.table_state.selected() else {
            return;
        };
        let m = state.music[selected].clone();
        Self::scan_music(m, state);
        Self::select_next(state);
    }
    fn scan_all(state: &mut State) {
        for m in state
            .music
            .clone()
            .into_iter()
            .filter(|x| state.filter.apply(x))
            .collect::<Vec<_>>()
        {
            if let Some(Lyrics::None) = state.lyrics.get(&m.path) {
                Self::scan_music(m, state);
            } else if let Some(Lyrics::Plain(_)) = state.lyrics.get(&m.path) {
                Self::scan_music(m, state);
            }
        }
    }
    fn scan_music(data: MusicData, state: &mut State) {
        let client = state.client.clone();
        let semaphore = state.client_limiter.clone();
        state.api_joins.spawn(async move {
            let Ok(lock) = semaphore.acquire_owned().await else {
                return LyricsRecord {
                    lyrics: Lyrics::None,
                    path: data.path,
                };
            };
            let lyrics = data.query(&client).await;
            drop(lock);
            LyricsRecord {
                lyrics,
                path: data.path,
            }
        });
        state.total += 1;
    }
    fn select_next(state: &mut State) {
        state.table_state.select_next();
    }

    fn select_previous(state: &mut State) {
        state.table_state.select_previous();
    }

    fn quit(state: &mut State) {
        state.will_quit = true;
    }
}

async fn get_or_create_config(state: &mut State) {
    if let Some(config_path) = default_config_path() {
        let config_file = tokio::fs::read_to_string(config_path)
            .await
            .unwrap_or_default();
        if let Ok(settings) = ron::from_str::<Settings>(config_file.as_str()) {
            Func::set_settings(state, settings).await;
        }
    } else if let Some(path) = default_future_config_path() {
        Func::set_settings(state, Settings::default()).await;
        if let Some(parent) = path.parent() {
            let Ok(()) = tokio::fs::create_dir_all(parent).await else {
                return;
            };
        } else {
            return;
        };
        let Ok(mut file) = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(path)
            .await
        else {
            return;
        };
        let settings = Settings::default();
        let ron = ron::to_string(&settings).unwrap();
        file.write_all(ron.as_bytes()).await.unwrap();
    }
}

#[tokio::main]
async fn main() {
    let mut terminal = ratatui::init();
    let mut keymap = std::collections::HashMap::<KeyBind, Func>::new();
    let mut state = State::default();
    for map in KEYMAP {
        keymap.insert(map.0, map.1);
    }
    let mut args = env::args();
    if let None = args.next() {};
    get_or_create_config(&mut state).await;

    loop {
        if state.total == state.done {
            state.total = 0;
            state.done = 0;
        }

        while let Some(Ok(log)) = state.api_joins.try_join_next() {
            log.save(&mut state);
            state.lyrics.insert(log.path, log.lyrics);
            state.done += 1;
        }
        while let Some(Ok(_)) = state.write_joins.try_join_next() {}

        let music = state.music.clone();
        let mut screen = Screen::default();
        screen.tracks = screen.tracks.rows(
            music
                .iter()
                .filter(|s| state.filter.apply(s))
                .map(|s| s.to_row()),
        );

        if let Err(e) = terminal.draw(|frame| {
            frame.render_stateful_widget(screen, frame.area(), &mut state);
        }) {
            println!("Error: {}", e);
            break;
        }

        if let Ok(true) = event::poll(Duration::from_millis(50)) {
            match event::read() {
                Ok(event) => state.event_handler(event, &keymap),
                Err(_) => {}
            }
        };

        if state.will_quit {
            break;
        }
    }
    ratatui::restore();
}

fn scan_music(path: PathBuf) -> Option<Vec<MusicData>> {
    let dir = std::fs::read_dir(path);
    let mut queue = VecDeque::new();
    let mut vec = Vec::new();
    queue.push_back(dir);
    while !queue.is_empty() {
        let Some(Ok(dir)) = queue.pop_front() else {
            continue;
        };

        for entry in dir {
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                queue.push_back(std::fs::read_dir(entry.path()));
            } else {
                let Some(path) = entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .and_then(|ext| {
                        if MUSIC_EXTENSIONS.contains(&ext) {
                            Some(entry.path())
                        } else {
                            None
                        }
                    })
                else {
                    continue;
                };
                vec.push(path);
            }
        }
    }
    let mut res = Vec::new();
    for path in vec {
        let Ok(data) = MusicData::from_file(path) else {
            continue;
        };
        res.push(data);
    }
    return Some(res);
}
