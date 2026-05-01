use std::{env, path::PathBuf};

use clap::Parser;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
    terminal,
};
use futures::{StreamExt, future};
use libc_constant_deprecator_lib::ConstContainer;
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender, error::TryRecvError},
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

#[expect(clippy::unused_async, unused, reason = "WIP.")]
pub(crate) async fn render(constants: ConstContainer, state: State) -> anyhow::Result<()> {
    loop {
        let State { events, mode, .. } = &mut state;

        // This is the part of the state machine that determines whether we should
        // transition to another state.
        match events.try_recv().map(|RawUserEvent { repr }| repr) {
            Ok(RawUserEventRepr::PlainText(c)) => todo!(),
            Ok(RawUserEventRepr::Return) => todo!(),
            Ok(RawUserEventRepr::ShiftReturn) => todo!(),
            Ok(RawUserEventRepr::Space) => todo!(),
            Ok(RawUserEventRepr::Escape) => todo!(),
            Err(TryRecvError::Empty) => {
                // If no new events have taken place, we display the same screen
                // state. The last state can one of the
                // following:
                // + The screen has some constants displayed on it,
            }
            Err(TryRecvError::Disconnected) => break,
        }
    }

    Ok(())
}

#[expect(clippy::unused_async, unused, reason = "WIP.")]
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let files = if let Some(path) = Args::parse().path {
        libc_constant_deprecator_lib::scan_files(PathBuf::from(path)).await?
    } else {
        libc_constant_deprecator_lib::scan_files(env::current_dir()?).await?
    };

    task::spawn_blocking(terminal::enable_raw_mode).await??;

    let mut parsed_constants = libc_constant_deprecator_lib::parse_constants(&files);
    let (mut state, events_tx) = State::new();

    let input_handler = task::spawn(handle_input(events_tx));
    let renderer = task::spawn(render(parsed_constants, state));

    match future::try_join(input_handler, renderer).await? {
        (Err(err), _) | (_, Err(err)) => Err(err),
        _ => Ok(()),
    }
}
