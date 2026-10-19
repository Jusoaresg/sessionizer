mod app;
mod config;
mod items;
mod scanner;
mod tmux;
mod ui;

use std::io;
use std::panic;

use crossterm::event::{self, Event};

use app::{ActionToExecute, App};

fn main() -> io::Result<()> {
    // Setup panic hook to cleanly restore terminal
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        ratatui::restore();
        original_hook(panic_info);
    }));

    let cfg = config::load_config();
    let mut app = App::new(cfg);

    let mut terminal = ratatui::init();

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Press {
                app.handle_key(key.code, key.modifiers);
            }
        }

        match &app.action {
            ActionToExecute::Launch { .. } | ActionToExecute::Exit => break,
            ActionToExecute::None => {}
        }
    }

    ratatui::restore();

    if let ActionToExecute::Launch {
        session_name,
        path,
        is_worktree,
        wt_name,
    } = app.action
    {
        tmux::launch_session(
            &session_name,
            &path,
            is_worktree,
            wt_name.as_deref(),
            &app.config.editor,
        );
    }

    Ok(())
}
