//! Terminal UI for upmix-core (ratatui + crossterm).
//!
//! Usage: `upmix-tui [input.flac]`
//!
//! Keys: Tab / ↑↓ switch parameter, ←→ adjust, Enter start, q quit.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};
use ratatui::{Frame, Terminal};

use upmix_core::{UpmixConfig, Upmixer};

enum Msg {
    Progress(usize, usize),
    Done(Result<PathBuf, String>),
}

struct Param {
    name: &'static str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    unit: &'static str,
}

struct App {
    input: Option<PathBuf>,
    params: Vec<Param>,
    sel: usize,
    status: String,
    progress: f32,
    running: bool,
    rx: Option<Receiver<Msg>>,
    started: Option<Instant>,
}

impl App {
    fn new(input: Option<PathBuf>) -> Self {
        let status = match &input {
            Some(p) => format!("Ready: {}", p.display()),
            None => "Usage: upmix-tui <input.flac>  (or pass a path)".to_owned(),
        };
        Self {
            input,
            params: vec![
                Param { name: "LFE gain", value: -6.0, min: -18.0, max: 6.0, step: 1.0, unit: "dB" },
                Param { name: "Surround gain", value: -3.0, min: -12.0, max: 0.0, step: 0.5, unit: "dB" },
                Param { name: "Surround delay", value: 12.0, min: 0.0, max: 30.0, step: 1.0, unit: "ms" },
                Param { name: "Vocal focus", value: 2.0, min: 0.0, max: 6.0, step: 0.5, unit: "dB" },
            ],
            sel: 0,
            status,
            progress: 0.0,
            running: false,
            rx: None,
            started: None,
        }
    }

    fn config(&self) -> UpmixConfig {
        let get = |i: usize| self.params[i].value;
        UpmixConfig {
            lfe_gain_db: get(0),
            surround_gain_db: get(1),
            surround_delay_ms: get(2),
            vocal_boost_db: get(3),
            ..Default::default()
        }
    }

    fn start(&mut self) {
        let Some(input) = self.input.clone() else {
            self.status = "No input file.".to_owned();
            return;
        };
        let cfg = self.config();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.running = true;
        self.progress = 0.0;
        self.started = Some(Instant::now());
        self.status = "Processing…".to_owned();
        std::thread::spawn(move || {
            let res = run_job(&input, cfg, tx.clone());
            let _ = tx.send(Msg::Done(res));
        });
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut done = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Progress(a, b) => self.progress = if b > 0 { a as f32 / b as f32 } else { 0.0 },
                Msg::Done(r) => done = Some(r),
            }
        }
        if let Some(r) = done {
            self.running = false;
            self.rx = None;
            self.progress = 1.0;
            let secs = self.started.take().map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
            self.status = match r {
                Ok(p) => format!("Done in {secs:.1}s → {}", p.display()),
                Err(e) => format!("Failed: {e}"),
            };
        }
    }
}

fn run_job(input: &std::path::Path, cfg: UpmixConfig, tx: Sender<Msg>) -> Result<PathBuf, String> {
    let buf = upmix_core::fileio::read_any(input).map_err(|e| e.to_string())?;
    if buf.num_channels() != 2 {
        return Err(format!("input is {}ch, need stereo", buf.num_channels()));
    }
    let out = Upmixer::new(cfg)
        .process_with_progress(&buf, |a, b| {
            let _ = tx.send(Msg::Progress(a, b));
        })
        .map_err(|e| e.to_string())?;
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
    let out_path = input.with_file_name(format!("{stem}_5.1.flac"));
    upmix_core::fileio::write_any(&out_path, &out, Some(input)).map_err(|e| e.to_string())?;
    Ok(out_path)
}

fn ui(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(app.params.len() as u16 + 2),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let title = Paragraph::new(format!(
        "Upmix → 5.1   input: {}",
        app.input
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".into())
    ))
    .block(Block::default().borders(Borders::ALL).title("upmix-tui"));
    frame.render_widget(title, chunks[0]);

    let mut lines = Vec::new();
    for (i, p) in app.params.iter().enumerate() {
        let marker = if i == app.sel { ">" } else { " " };
        let style = if i == app.sel {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker} {:<16} {:>7.1} {}", p.name, p.value, p.unit),
            style,
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Parameters")),
        chunks[1],
    );

    frame.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("Progress"))
            .gauge_style(Style::default().fg(Color::Green))
            .ratio(app.progress.clamp(0.0, 1.0) as f64),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(app.status.clone()).block(Block::default().borders(Borders::ALL).title("Status")),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new("Tab/↑↓ select · ←→ adjust · Enter start · r rescan? · q quit"),
        chunks[4],
    );
}

fn main() -> anyhow::Result<()> {
    let input = std::env::args().nth(1).map(PathBuf::from);
    let mut app = App::new(input);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(stdout))?;

    let res = run(&mut term, &mut app);

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    res
}

fn run(term: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> anyhow::Result<()> {
    loop {
        app.poll();
        term.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(120))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Tab | KeyCode::Down => {
                        app.sel = (app.sel + 1) % app.params.len();
                    }
                    KeyCode::Up => {
                        app.sel = (app.sel + app.params.len() - 1) % app.params.len();
                    }
                    KeyCode::Left => {
                        let p = &mut app.params[app.sel];
                        p.value = (p.value - p.step).max(p.min);
                    }
                    KeyCode::Right => {
                        let p = &mut app.params[app.sel];
                        p.value = (p.value + p.step).min(p.max);
                    }
                    KeyCode::Enter => {
                        if !app.running {
                            app.start();
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
