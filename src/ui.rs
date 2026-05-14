use crate::app::{format_duration, format_views, App, AppState};
use crate::config::{Config, QUALITY_OPTIONS};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use ratatui_image::StatefulImage;

// Catppuccin Mocha
const BG: Color = Color::Rgb(17, 17, 27);
const FG: Color = Color::Rgb(205, 214, 244);
const DIM: Color = Color::Rgb(108, 112, 134);
const ACCENT: Color = Color::Rgb(180, 190, 254);
const GOLD: Color = Color::Rgb(249, 226, 175);
const GREEN: Color = Color::Rgb(166, 227, 161);
const RED: Color = Color::Rgb(243, 139, 168);
const SURFACE: Color = Color::Rgb(49, 50, 68);

const SPINNER: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];

pub fn draw(f: &mut Frame, app: &mut App, config: &Config) {
    let size = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), size);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(1),   // content
            Constraint::Length(1), // status bar
        ])
        .split(size);

    draw_header(f, app, config, chunks[0]);

    match app.state {
        AppState::Loading => draw_loading(f, app, chunks[1]),
        AppState::Channels => draw_channels(f, app, config, chunks[1]),
        AppState::Feed | AppState::Search | AppState::Quality => draw_content(f, app, config, chunks[1]),
        AppState::Error => draw_error(f, app, chunks[1]),
    }

    draw_status_bar(f, app, chunks[2]);

    if app.state == AppState::Quality {
        draw_quality_picker(f, app, size);
    }
}

fn draw_header(f: &mut Frame, app: &App, config: &Config, area: Rect) {
    let title = match app.state {
        AppState::Search => " search ",
        AppState::Channels => " channels ",
        _ => " cathode",
    };

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(SURFACE))
        .style(Style::default().bg(BG));

    if app.state == AppState::Search {
        let input = Paragraph::new(Line::from(vec![
            Span::styled(&app.search_input, Style::default().fg(FG)),
            Span::styled(
                "_",
                Style::default()
                    .fg(ACCENT)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]))
        .block(block);
        f.render_widget(input, area);
    } else if app.state == AppState::Channels {
        let info = Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{} followed", config.channels.len()),
                Style::default().fg(DIM),
            ),
        ]))
        .block(block);
        f.render_widget(info, area);
    } else {
        let feed_label = match &app.feed_kind {
            crate::app::FeedKind::Trending => "trending".to_string(),
            crate::app::FeedKind::Subscriptions => "subscriptions".to_string(),
            crate::app::FeedKind::Curated(channels) => channels.join(" · "),
        };

        let mut spans = vec![
            Span::styled(&feed_label, Style::default().fg(DIM)),
            Span::styled(
                format!("  {} videos", app.videos.len()),
                Style::default().fg(DIM),
            ),
        ];

        // Follow flash
        if let Some((ref msg, _)) = app.follow_flash {
            spans.push(Span::styled("  ", Style::default().fg(DIM)));
            spans.push(Span::styled(msg, Style::default().fg(GREEN)));
        }

        let info = Paragraph::new(Line::from(spans)).block(block);
        f.render_widget(info, area);
    }
}

fn draw_loading(f: &mut Frame, app: &App, area: Rect) {
    let spinner = SPINNER[app.spinner_frame];
    let text = Paragraph::new(Line::from(vec![
        Span::styled(spinner, Style::default().fg(ACCENT)),
        Span::styled(format!(" {}", app.status_msg), Style::default().fg(DIM)),
    ]))
    .style(Style::default().bg(BG));
    f.render_widget(text, area);
}

fn draw_content(f: &mut Frame, app: &mut App, config: &Config, area: Rect) {
    let has_thumbnail = app.thumbnail.is_some() && app.picker.is_some();
    let thumb_width = if has_thumbnail && area.width > 60 {
        (area.width * 30 / 100).clamp(20, 40)
    } else {
        0
    };

    if thumb_width > 0 {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(thumb_width),
            ])
            .split(area);

        draw_video_list(f, app, config, chunks[0]);
        draw_thumbnail(f, app, chunks[1]);
    } else {
        draw_video_list(f, app, config, area);
    }
}

fn draw_video_list(f: &mut Frame, app: &App, config: &Config, area: Rect) {
    if app.state == AppState::Search && app.videos.is_empty() {
        let hint = Paragraph::new(Line::from(vec![
            Span::styled("type a query and press ", Style::default().fg(DIM)),
            Span::styled("Enter", Style::default().fg(ACCENT)),
        ]))
        .style(Style::default().bg(BG));
        f.render_widget(hint, area);
        return;
    }

    let visible_height = area.height as usize;

    let scroll = if app.selected >= app.scroll_offset + visible_height {
        app.selected - visible_height + 1
    } else if app.selected < app.scroll_offset {
        app.selected
    } else {
        app.scroll_offset
    };

    let items: Vec<ListItem> = app
        .videos
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, video)| {
            let is_selected = i == app.selected;
            let is_followed = !video.channel_id.is_empty() && config.has_channel(&video.channel_id);
            let duration = format_duration(video.duration_secs);
            let views = format_views(video.views);

            let title_style = if is_selected {
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };

            let meta = format!("{} · {} · {}", video.author, duration, views);
            let meta_with_age = if video.published_text.is_empty() {
                meta
            } else {
                format!("{} · {}", meta, video.published_text)
            };

            let indicator = if is_selected { "▸ " } else { "  " };

            let mut title_spans = vec![
                Span::styled(indicator, Style::default().fg(ACCENT)),
                Span::styled(&video.title, title_style),
            ];
            if is_followed {
                title_spans.push(Span::styled(" +", Style::default().fg(GREEN)));
            }

            ListItem::new(vec![
                Line::from(title_spans),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(meta_with_age, Style::default().fg(DIM)),
                ]),
            ])
        })
        .collect();

    let list = List::new(items).style(Style::default().bg(BG));
    f.render_widget(list, area);
}

fn draw_thumbnail(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(SURFACE))
        .style(Style::default().bg(BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(ref mut proto) = app.thumbnail {
        let image = StatefulImage::default();
        f.render_stateful_widget(image, inner, proto);
    }
}

fn draw_channels(f: &mut Frame, app: &App, config: &Config, area: Rect) {
    if config.channels.is_empty() {
        let hint = Paragraph::new(Line::from(vec![
            Span::styled("no channels followed. press ", Style::default().fg(DIM)),
            Span::styled("f", Style::default().fg(ACCENT)),
            Span::styled(" on a video to follow.", Style::default().fg(DIM)),
        ]))
        .style(Style::default().bg(BG));
        f.render_widget(hint, area);
        return;
    }

    let visible_height = area.height as usize;

    let scroll = if app.channel_selected >= app.channel_scroll + visible_height {
        app.channel_selected - visible_height + 1
    } else if app.channel_selected < app.channel_scroll {
        app.channel_selected
    } else {
        app.channel_scroll
    };

    let items: Vec<ListItem> = config
        .channels
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, ch)| {
            let is_selected = i == app.channel_selected;
            let indicator = if is_selected { "▸ " } else { "  " };
            let style = if is_selected {
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(Line::from(vec![
                Span::styled(indicator, Style::default().fg(ACCENT)),
                Span::styled(&ch.name, style),
                Span::styled(format!("  {}", ch.id), Style::default().fg(DIM)),
            ]))
        })
        .collect();

    let list = List::new(items).style(Style::default().bg(BG));
    f.render_widget(list, area);
}

fn draw_error(f: &mut Frame, app: &App, area: Rect) {
    let text = Paragraph::new(Line::from(vec![
        Span::styled(
            "error: ",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(&app.error_msg, Style::default().fg(FG)),
    ]))
    .style(Style::default().bg(BG));
    f.render_widget(text, area);
}

fn draw_quality_picker(f: &mut Frame, app: &App, area: Rect) {
    let picker_h = (QUALITY_OPTIONS.len() as u16) + 2;
    let picker_w = 22;
    let x = area.width.saturating_sub(picker_w) / 2;
    let y = area.height.saturating_sub(picker_h) / 2;
    let popup = Rect::new(x, y, picker_w, picker_h);

    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            " quality ",
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BG));

    let items: Vec<ListItem> = QUALITY_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, (_, label))| {
            let is_selected = i == app.quality_selected;
            let indicator = if is_selected { "▸ " } else { "  " };
            let style = if is_selected {
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(Line::from(vec![
                Span::styled(indicator, Style::default().fg(ACCENT)),
                Span::styled(*label, style),
            ]))
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, popup);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.state {
        AppState::Loading => vec![("", "loading...")],
        AppState::Feed => vec![
            ("j/k", "navigate"),
            ("Enter", "play"),
            ("f", "follow"),
            ("c", "channels"),
            ("/", "search"),
            ("r", "refresh"),
            ("q", "quit"),
        ],
        AppState::Search => vec![("Enter", "search"), ("Esc", "back")],
        AppState::Quality => vec![("j/k", "select"), ("Enter", "play"), ("Esc", "back")],
        AppState::Channels => vec![("j/k", "navigate"), ("d", "remove"), ("Esc", "back")],
        AppState::Error => vec![("r", "retry"), ("q", "quit")],
    };

    let spans: Vec<Span> = hints
        .iter()
        .enumerate()
        .flat_map(|(i, (key, desc))| {
            let mut s = vec![
                Span::styled(
                    *key,
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {}", desc), Style::default().fg(DIM)),
            ];
            if i < hints.len() - 1 {
                s.push(Span::styled("  ", Style::default().fg(DIM)));
            }
            s
        })
        .collect();

    let bar = Paragraph::new(Line::from(spans)).style(Style::default().bg(SURFACE));
    f.render_widget(bar, area);
}
