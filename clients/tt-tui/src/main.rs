//! Turing Tarpit terminal client — `tt`
//!
//! Theme: pitch black + accent `#ff3c00` (Turing orange).

mod api;
mod app;
mod ui;

use std::io::{self, stdout};

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use app::{App, Screen};
use ui::draw;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(io::stderr)
        .try_init();

    let mut app = App::load().await?;

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| {
            app.frame_area = f.area();
            draw(f, app);
        })?;

        if event::poll(std::time::Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') if !app.input_mode() => {
                            if app.screen == Screen::Login || app.screen == Screen::Home {
                                break;
                            }
                            app.screen = Screen::Home;
                        }
                        KeyCode::Char('c')
                            if key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL) =>
                        {
                            break;
                        }
                        other => app.handle_key(other).await?,
                    }
                }
                Event::Mouse(m) => {
                    // Treat press + touch-down as a tap
                    let is_tap = matches!(
                        m.kind,
                        MouseEventKind::Down(MouseButton::Left)
                            | MouseEventKind::Up(MouseButton::Left)
                    );
                    // Prefer Down so we don't double-fire on Up
                    if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                        app.handle_mouse(m.column, m.row).await?;
                    } else if is_tap {
                        // ignore Up
                    } else if matches!(m.kind, MouseEventKind::ScrollUp) {
                        app.handle_key(KeyCode::Up).await?;
                    } else if matches!(m.kind, MouseEventKind::ScrollDown) {
                        app.handle_key(KeyCode::Down).await?;
                    }
                }
                Event::Resize(_, _) => {
                    // next draw picks up new area
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
