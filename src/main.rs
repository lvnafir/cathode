mod api;
mod app;
mod config;
mod feed;
mod history;
mod ui;

use app::{App, AppState, FeedKind};
use api::{Video, YtdlpSource, YouTubeSource};
use config::Config;
use history::History;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::process::Command;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

enum WorkerMsg {
    Videos(Vec<Video>, FeedKind),
    Error(String),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load();
    let mut history = History::load();

    let source = Arc::new(YtdlpSource);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let mut rx: Option<mpsc::Receiver<WorkerMsg>> = Some(spawn_fetch_feed(&config, &source, &history));

    loop {
        app.tick();

        // Poll worker
        if let Some(ref receiver) = rx {
            match receiver.try_recv() {
                Ok(WorkerMsg::Videos(videos, kind)) => {
                    app.feed_kind = kind;
                    app.set_videos(videos);
                    rx = None;
                }
                Ok(WorkerMsg::Error(e)) => {
                    app.set_error(e);
                    rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    app.set_error("worker disconnected".into());
                    rx = None;
                }
            }
        }

        terminal.draw(|f| ui::draw(f, &mut app, &config))?;

        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Global quit
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
                {
                    break;
                }

                match app.state {
                    AppState::Loading => {
                        if key.code == KeyCode::Char('q') {
                            break;
                        }
                    }
                    AppState::Feed => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('j') | KeyCode::Down => app.move_down(),
                        KeyCode::Char('k') | KeyCode::Up => app.move_up(),
                        KeyCode::Char('/') => app.enter_search(),
                        KeyCode::Char('f') => app.toggle_follow(&mut config),
                        KeyCode::Char('c') => app.enter_channels(),
                        KeyCode::Char('r') => {
                            app.state = AppState::Loading;
                            app.status_msg = "refreshing...".into();
                            rx = Some(spawn_fetch_feed(&config, &source, &history));
                        }
                        KeyCode::Enter => {
                            app.enter_quality(config.max_resolution);
                        }
                        _ => {}
                    },
                    AppState::Quality => match key.code {
                        KeyCode::Esc => app.exit_quality(),
                        KeyCode::Char('j') | KeyCode::Down => app.quality_down(),
                        KeyCode::Char('k') | KeyCode::Up => app.quality_up(),
                        KeyCode::Enter => {
                            let url = app.quality_video_url.clone();
                            let video_info = app.quality_video_info.clone();
                            let height = app.selected_quality();
                            let ytdl_format = format!(
                                "bv*[height<={}]+ba/b[height<={}]",
                                height, height
                            );
                            app.exit_quality();

                            disable_raw_mode()?;
                            execute!(
                                terminal.backend_mut(),
                                LeaveAlternateScreen
                            )?;

                            let win = hide_terminal();

                            let watch_start = std::time::Instant::now();
                            let _ = Command::new("mpv")
                                .arg(&format!("--ytdl-format={}", ytdl_format))
                                .arg(&url)
                                .status();
                            let watch_secs = watch_start.elapsed().as_secs();

                            // Log watch
                            if let Some((vid, ch_id, ch_name, title)) = video_info {
                                history.log_watch(&vid, &ch_id, &ch_name, &title, "", watch_secs);
                            }

                            show_terminal(win);

                            execute!(io::stdout(), EnterAlternateScreen)?;
                            enable_raw_mode()?;
                            terminal.clear()?;
                            app.reinit_picker();
                        }
                        _ => {}
                    },
                    AppState::Channels => match key.code {
                        KeyCode::Esc | KeyCode::Char('c') => app.exit_channels(),
                        KeyCode::Char('j') | KeyCode::Down => app.channel_down(config.channels.len()),
                        KeyCode::Char('k') | KeyCode::Up => app.channel_up(),
                        KeyCode::Char('d') | KeyCode::Delete => {
                            if !config.channels.is_empty() {
                                let id = config.channels[app.channel_selected].id.clone();
                                config.unfollow(&id);
                                if app.channel_selected >= config.channels.len() && app.channel_selected > 0 {
                                    app.channel_selected -= 1;
                                }
                            }
                        }
                        _ => {}
                    },
                    AppState::Search => match key.code {
                        KeyCode::Esc => app.exit_search(),
                        KeyCode::Enter => {
                            if !app.search_input.is_empty() {
                                let query = app.search_input.clone();
                                app.state = AppState::Loading;
                                app.status_msg = format!("searching '{}'...", query);
                                rx = Some(spawn_search(&source, &query));
                            }
                        }
                        KeyCode::Backspace => app.search_backspace(),
                        KeyCode::Char(c) => app.search_insert(c),
                        _ => {}
                    },
                    AppState::Error => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('r') => {
                            app.state = AppState::Loading;
                            app.status_msg = "retrying...".into();
                            rx = Some(spawn_fetch_feed(&config, &source, &history));
                        }
                        _ => {}
                    },
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn spawn_fetch_feed(config: &Config, source: &Arc<YtdlpSource>, history: &History) -> mpsc::Receiver<WorkerMsg> {
    let (tx, rx) = mpsc::channel();
    let config = config.clone();
    let source = Arc::clone(source);
    // Precompute history data so we don't need to send History across threads
    let affinity = history.channel_affinity();
    let topic_keywords = history.topic_keywords(5);
    thread::spawn(move || {
        // Try subscriptions CSV first
        let subs = feed::load_subscriptions();
        if !subs.is_empty() {
            let videos = feed::fetch_subscription_feed(&source, &subs);
            if !videos.is_empty() {
                let _ = tx.send(WorkerMsg::Videos(videos, FeedKind::Subscriptions));
                return;
            }
        }

        // Curated feed: random sample of channels + search queries (parallel)
        if !config.channels.is_empty() {
            let result = feed::fetch_curated_feed(&source, &config, &affinity, &topic_keywords);
            if !result.videos.is_empty() {
                let _ = tx.send(WorkerMsg::Videos(result.videos, FeedKind::Curated(result.sampled_channels)));
                return;
            }
        }

        // Last resort: plain trending
        match source.trending(&config.region) {
            Ok(videos) => {
                let _ = tx.send(WorkerMsg::Videos(videos, FeedKind::Trending));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::Error(format!("{}", e)));
            }
        }
    });
    rx
}

enum TermWindow {
    Hyprland { address: String, workspace: String },
    Sway,
    X11 { wid: String },
    None,
}

fn hide_terminal() -> TermWindow {
    // Hyprland: get active window, stash its address + workspace, move to special
    if let Ok(output) = Command::new("hyprctl").args(["activewindow", "-j"]).output() {
        if output.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                let addr = val["address"].as_str().unwrap_or("").to_string();
                let ws = val["workspace"]["name"].as_str().unwrap_or("1").to_string();
                if !addr.is_empty() {
                    let _ = Command::new("hyprctl")
                        .args(["dispatch", "movetoworkspacesilent", &format!("special:cathode,address:{}", addr)])
                        .output();
                    return TermWindow::Hyprland { address: addr, workspace: ws };
                }
            }
        }
    }

    // Sway/dwl-with-ipc: move focused to scratchpad
    if let Ok(output) = Command::new("swaymsg").args(["-t", "get_tree"]).output() {
        if output.status.success() {
            if let Ok(output) = Command::new("swaymsg").args(["move", "scratchpad"]).output() {
                if output.status.success() {
                    return TermWindow::Sway;
                }
            }
        }
    }

    // X11 (dwm, i3, openbox, anything): unmap the window entirely
    if let Ok(output) = Command::new("xdotool").args(["getactivewindow"]).output() {
        if output.status.success() {
            let wid = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !wid.is_empty() {
                let _ = Command::new("xdotool").args(["windowunmap", "--sync", &wid]).status();
                return TermWindow::X11 { wid };
            }
        }
    }

    TermWindow::None
}

fn show_terminal(win: TermWindow) {
    match win {
        TermWindow::Hyprland { address, workspace } => {
            let _ = Command::new("hyprctl")
                .args(["dispatch", "movetoworkspacesilent", &format!("{},address:{}", workspace, address)])
                .output();
            let _ = Command::new("hyprctl")
                .args(["dispatch", "focuswindow", &format!("address:{}", address)])
                .output();
        }
        TermWindow::Sway => {
            let _ = Command::new("swaymsg").args(["scratchpad", "show"]).output();
        }
        TermWindow::X11 { wid } => {
            let _ = Command::new("xdotool").args(["windowmap", &wid]).status();
            let _ = Command::new("xdotool").args(["windowactivate", &wid]).status();
        }
        TermWindow::None => {}
    }
}

fn spawn_search(source: &Arc<YtdlpSource>, query: &str) -> mpsc::Receiver<WorkerMsg> {
    let (tx, rx) = mpsc::channel();
    let source = Arc::clone(source);
    let query = query.to_string();
    thread::spawn(move || {
        match source.search(&query) {
            Ok(videos) => {
                let _ = tx.send(WorkerMsg::Videos(videos, FeedKind::Trending));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::Error(format!("{}", e)));
            }
        }
    });
    rx
}
