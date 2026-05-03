#![expect(unused, reason = "WIP.")]

use std::{
    env,
    io::{self, Write},
    path::PathBuf,
};

use clap::Parser;
use crossterm::{
    cursor::{self, MoveToNextLine, MoveToRow, SetCursorStyle},
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
    terminal::{self, Clear, ClearType},
};
use futures::{StreamExt, future};
use libc_constant_deprecator_lib::{BorrowedContainer, ConstContainer};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task,
};

#[derive(Debug, Parser)]
pub(crate) struct Args {
    path: Option<String>,
}

#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct State {
    events: UnboundedReceiver<RawUserEvent>,
    mode: Mode,
}

impl State {
    pub(crate) fn new() -> (Self, UnboundedSender<RawUserEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();

        (
            Self {
                events: rx,
                mode: Mode::default(),
            },
            tx,
        )
    }
}

#[derive(Debug, Default)]
pub(crate) struct Mode {
    repr: ModeRepr,
}

impl Mode {
    pub(crate) fn repr(&self) -> &ModeRepr {
        &self.repr
    }
}

#[derive(Debug, Default)]
pub(crate) enum ModeRepr {
    Insert,
    #[default]
    Normal,
}

#[derive(Debug)]
pub(crate) struct UserEvent {
    repr: UserEventRepr,
}

impl UserEvent {
    pub(crate) fn new(event: UserEventRepr) -> Self {
        Self { repr: event }
    }
}

#[derive(Debug)]
pub(crate) enum UserEventRepr {
    // Corresponds with plain text user input at the prompt.
    TextualInput(String),
    // Is triggered with the return key and should trigger a filtering event with the last saved
    // regex input at the prompt.
    Search,
    // Is triggered with the space key and should toggle all selected constants' state to
    // "deprecated", unless all selected constants are already deprecated, in which case it should
    // undeprecate.
    Toggle,
    // Is triggered with the shift + return combo and should effect the changes to disk.
    Effect,
    // Is triggered with the escape key and should clear the currently input regex.
    Clear,
}

#[derive(Debug)]
pub(crate) struct RawUserEvent {
    repr: RawUserEventRepr,
}

impl RawUserEvent {
    pub(crate) fn new(event: RawUserEventRepr) -> Self {
        Self { repr: event }
    }
}

#[derive(Debug)]
pub(crate) enum RawUserEventRepr {
    PlainText(char),
    Return,
    Space,
    ShiftReturn,
    Escape,
}

#[expect(clippy::unused_async, unused, reason = "WIP.")]
pub(crate) async fn render(mut constants: ConstContainer, mut state: State) -> anyhow::Result<()> {
    // TODO: possibly make `BorrowedContainer` hold `Weak`s and probe into the
    // validty of the `ConstContainer` it sources from by `upgrade()`ing and only
    // then actually mutating. It holds that the `BorrowedContainer` we have here
    // will never have a longer lifetime than the `ConstContainer`, but it's better
    // to be safe than to be sorry.
    let mut filter_buf = constants.borrowed();

    loop {
        constants.filter_with("", &mut filter_buf)?;
        draw_screen(&mut filter_buf, &mut state);
        update();
    }
}

pub(crate) fn draw_screen(buf: &mut BorrowedContainer, state: &mut State) {}

pub(crate) fn update() {}

pub(crate) async fn handle_input(channel: UnboundedSender<RawUserEvent>) -> anyhow::Result<()> {
    let mut event_stream = EventStream::new().fuse();

    while let Some(event) = event_stream.next().await {
        match event? {
            Event::Key(KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::new(RawUserEventRepr::Space)),
            Event::Key(KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::new(RawUserEventRepr::PlainText(c))),
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::new(RawUserEventRepr::Return)),
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::SHIFT,
                ..
            }) => _ = channel.send(RawUserEvent::new(RawUserEventRepr::ShiftReturn)),
            Event::Key(KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::new(RawUserEventRepr::Escape)),
            _ => (),
        }
    }

    Ok(())
}

pub(crate) fn prepare_space() -> anyhow::Result<()> {
    let (rows, _) = terminal::size()?;
    let (current_row, _) = cursor::position()?;

    let mut stdout = io::stdout().lock();

    crossterm::queue!(stdout, SetCursorStyle::SteadyBlock)?;

    // NOTE: we must make space for one row for the prompt, and ten rows for the
    // list of constants currently being displayed. The reason why we use one more
    // unit in the result of the difference than in the command to set the row is
    // due to the fact `rows` is 1-indexed, while `current_row` and `MoveToRow` are
    // 0-indexed.
    if rows - current_row < 12 {
        crossterm::queue!(stdout, MoveToRow(current_row - 11))?;
        crossterm::queue!(stdout, Clear(ClearType::FromCursorDown))?;
        crossterm::queue!(stdout, MoveToNextLine(1))?;
    }

    stdout.flush()?;

    Ok(())
}

// FIXME: disable raw mode on fallible expressions beyond those that come before
// it's enabled and while enabling it. This could benefit from the `defer_drm`
// proc-macro that we developed in the `tester-impl` crate for `tester`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let files = if let Some(path) = Args::parse().path {
        libc_constant_deprecator_lib::scan_files(PathBuf::from(path)).await?
    } else {
        libc_constant_deprecator_lib::scan_files(env::current_dir()?).await?
    };

    task::spawn_blocking(terminal::enable_raw_mode).await??;

    let parsed_constants = libc_constant_deprecator_lib::parse_constants(&files);
    let (state, events_tx) = State::new();

    task::spawn_blocking(prepare_space).await??;

    let input_handler = task::spawn(handle_input(events_tx));
    let renderer = task::spawn(render(parsed_constants, state));

    future::try_join(input_handler, renderer)
        .await
        .map(|(res1, res2)| res1.and(res2))?
}
