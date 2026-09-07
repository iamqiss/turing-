//! Ratatui widgets — black + Turing orange `#ff3c00`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, AuthMode, LoginField, Modal, RepoField, Screen, WsField};

const ORANGE: Color = Color::Rgb(255, 60, 0);
const BG: Color = Color::Rgb(10, 10, 12);
const MUTED: Color = Color::Rgb(140, 140, 150);
const FG: Color = Color::Rgb(230, 230, 235);

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(
        Block::default().style(Style::default().bg(BG).fg(FG)),
        area,
    );

    match app.screen {
        Screen::Login => draw_login(f, app, area),
        Screen::Repos => draw_repos(f, app, area),
        _ => draw_shell(f, app, area),
    }
}

fn draw_login(f: &mut Frame, app: &App, area: Rect) {
    let form_h = match app.auth_mode {
        AuthMode::SignIn => 14u16,
        AuthMode::Register => 18u16,
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(18),
            Constraint::Length(form_h),
            Constraint::Min(3),
        ])
        .split(area);

    let title = Paragraph::new(vec![
        Line::from(Span::styled(
            "  TURING TARPIT",
            Style::default()
                .fg(ORANGE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  terminal client",
            Style::default().fg(MUTED),
        )),
    ])
    .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(title, chunks[0]);

    let box_w = 60u16.min(area.width.saturating_sub(4));
    let box_x = area.x + (area.width.saturating_sub(box_w)) / 2;
    let form_area = Rect {
        x: box_x,
        y: chunks[1].y,
        width: box_w,
        height: chunks[1].height,
    };

    let handle_style = field_style(app.login_field == LoginField::Handle);
    let name_style = field_style(app.login_field == LoginField::DisplayName);
    let pass_style = field_style(app.login_field == LoginField::Passphrase);
    let server_style = field_style(app.login_field == LoginField::Server);
    let stars = "*".repeat(app.passphrase.chars().count());

    let mut lines = vec![
        Line::from(Span::styled(" handle", Style::default().fg(MUTED))),
        Line::from(Span::styled(format!(" {}", app.handle), handle_style)),
        Line::from(""),
    ];
    if app.auth_mode == AuthMode::Register {
        lines.push(Line::from(Span::styled(
            " display name",
            Style::default().fg(MUTED),
        )));
        lines.push(Line::from(Span::styled(
            format!(" {}", app.display_name),
            name_style,
        )));
        lines.push(Line::from(""));
    }
    lines.extend([
        Line::from(Span::styled(" passphrase", Style::default().fg(MUTED))),
        Line::from(Span::styled(format!(" {stars}"), pass_style)),
        Line::from(""),
        Line::from(Span::styled(" server", Style::default().fg(MUTED))),
        Line::from(Span::styled(format!(" {}", app.server), server_style)),
        Line::from(""),
        Line::from(Span::styled(
            match app.auth_mode {
                AuthMode::SignIn => " Tab fields · Enter sign in · F2 create account · Ctrl-C quit",
                AuthMode::Register => {
                    " Tab fields · Enter create account · F2 sign in · Ctrl-C quit"
                }
            },
            Style::default().fg(MUTED),
        )),
    ]);

    let title = match app.auth_mode {
        AuthMode::SignIn => " sign in ",
        AuthMode::Register => " create account ",
    };
    let form = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ORANGE))
            .title(Span::styled(title, Style::default().fg(ORANGE))),
    );
    f.render_widget(form, form_area);

    let mut status_lines = vec![Line::from(Span::styled(
        &app.status,
        Style::default().fg(MUTED),
    ))];
    if let Some(err) = &app.error {
        status_lines.push(Line::from(Span::styled(
            err.as_str(),
            Style::default().fg(Color::Red),
        )));
    }
    f.render_widget(
        Paragraph::new(status_lines).alignment(ratatui::layout::Alignment::Center),
        chunks[2],
    );
}

fn field_style(active: bool) -> Style {
    if active {
        Style::default()
            .fg(FG)
            .bg(Color::Rgb(40, 20, 10))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(FG)
    }
}

fn draw_top_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = App::menu_items()
        .iter()
        .map(|(label, screen)| {
            let selected = app.screen == *screen;
            Line::from(Span::styled(
                format!(" {label} "),
                if selected {
                    Style::default()
                        .fg(ORANGE)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(MUTED)
                },
            ))
        })
        .collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::Rgb(40, 40, 48)))
                .title(Span::styled(
                    " Turing Tarpit ",
                    Style::default()
                        .fg(ORANGE)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .select(
            App::menu_items()
                .iter()
                .position(|(_, s)| *s == app.screen)
                .unwrap_or(0),
        )
        .divider(Span::raw("│"));
    f.render_widget(tabs, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" status ", Style::default().fg(MUTED)),
        Span::styled(&app.status, Style::default().fg(FG)),
        Span::raw("  "),
        Span::styled(
            app.error.as_deref().unwrap_or(""),
            Style::default().fg(Color::Red),
        ),
    ]));
    f.render_widget(footer, area);
}

fn draw_repos(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);

    draw_top_tabs(f, app, chunks[0]);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    let items: Vec<ListItem> = if app.repos.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (no repositories)",
            Style::default().fg(MUTED),
        ))]
    } else {
        app.repos
            .iter()
            .map(|row| {
                let vis = if row.repo.is_private { "🔒" } else { "  " };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(" {} ", vis),
                        Style::default().fg(MUTED),
                    ),
                    Span::styled(
                        format!("{}/", row.workspace_slug),
                        Style::default().fg(MUTED),
                    ),
                    Span::styled(
                        row.repo.name.clone(),
                        Style::default().fg(FG).add_modifier(Modifier::BOLD),
                    ),
                ]))
            })
            .collect()
    };

    let mut state = ListState::default();
    if !app.repos.is_empty() {
        state.select(Some(app.list_idx));
    }

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 58)))
                .title(Span::styled(" repositories ", Style::default().fg(ORANGE))),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 20, 10))
                .fg(ORANGE)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    f.render_stateful_widget(list, mid[0], &mut state);

    let detail = match &app.modal {
        Modal::CreateWorkspace { name, slug, field } => {
            let n = if *field == WsField::Name { ">" } else { " " };
            let s = if *field == WsField::Slug { ">" } else { " " };
            format!(
                "New workspace

{n} name: {name}
{s} slug: {slug}

Tab fields · Enter create · Esc cancel
Slug auto-fills from name until you edit it."
            )
        }
        Modal::CreateRepo {
            name,
            description,
            is_private,
            field,
        } => {
            let ws = app
                .workspaces
                .get(app.selected_workspace)
                .map(|w| w.slug.as_str())
                .unwrap_or("?");
            let n = if *field == RepoField::Name { ">" } else { " " };
            let d = if *field == RepoField::Description { ">" } else { " " };
            let v = if *field == RepoField::Visibility { ">" } else { " " };
            let vis = if *is_private { "private" } else { "public" };
            format!(
                "New repository in {ws}

{n} name: {name}
{d} description: {description}
{v} visibility: {vis}  (Space/p toggle)

Tab fields · Enter create · Esc cancel"
            )
        }
        Modal::ConfirmDeleteRepo => {
            if let Some(row) = app.repos.get(app.list_idx) {
                format!(
                    "Delete repository?

  {}/{}

This cannot be undone from the TUI.

y / Enter confirm · n / Esc cancel",
                    row.workspace_slug, row.repo.name
                )
            } else {
                "Nothing to delete".into()
            }
        }
        Modal::None => {
            if let Some(row) = app.repos.get(app.list_idx) {
                format!(
                    "{}/{}

{}

private: {}
default: {}
clone: {}{}
id: {}
workspace: {}

j/k move · n new repo · w new workspace · d delete · [ ] workspace · r refresh",
                    row.workspace_slug,
                    row.repo.name,
                    row.repo.description.as_deref().unwrap_or("No description"),
                    row.repo.is_private,
                    row.repo.default_branch,
                    app.server.trim_end_matches('/'),
                    row.repo.clone_url_path,
                    row.repo.id,
                    row.workspace_name,
                )
            } else {
                app.body.clone()
            }
        }
    };


    let detail_w = Paragraph::new(detail)
        .style(Style::default().fg(FG))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 58)))
                .title(Span::styled(" detail ", Style::default().fg(ORANGE))),
        );
    f.render_widget(detail_w, mid[1]);

    draw_footer(f, app, chunks[2]);
}

fn draw_shell(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);

    draw_top_tabs(f, app, chunks[0]);

    let body = Paragraph::new(app.body.as_str())
        .style(Style::default().fg(FG))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 58)))
                .title(Span::styled(
                    match app.screen {
                        Screen::Home => " home ",
                        Screen::Inbox => " inbox ",
                        Screen::Channels => " channels ",
                        Screen::Compute => " compute ",
                        Screen::Settings => " settings ",
                        _ => " ",
                    },
                    Style::default().fg(ORANGE),
                )),
        );
    f.render_widget(body, chunks[1]);
    draw_footer(f, app, chunks[2]);
}
