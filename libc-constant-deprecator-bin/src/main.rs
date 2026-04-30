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
}

impl State {
    pub(crate) fn new(events_channel: UnboundedReceiver<RawUserEvent>) -> Self {
        Self {
            events: events_channel,
        }
    }
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
    ShiftReturn,
    Space,
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

#[expect(clippy::unused_async, reason = "WIP.")]
pub(crate) async fn render(constants: ConstContainer, state: State) -> anyhow::Result<()> {
    // Every time the rendering loop iterates, it ought send the current state of
    // affairs within `state`, which contains information on the current input mode
    // (either insert or normal mode.)
    loop {
        let State { events, .. } = &mut state;

        match events.try_recv().map(|RawUserEvent { repr }| repr) {
            Ok(RawUserEventRepr::PlainText(c)) => todo!(),
            Ok(RawUserEventRepr::Return) => todo!(),
            Ok(RawUserEventRepr::ShiftReturn) => todo!(),
            Ok(RawUserEventRepr::Space) => todo!(),
            Ok(RawUserEventRepr::Escape) => todo!(),
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Disconnected) => break,
        }
    }

    Ok(())
}

#[expect(clippy::unused_async, reason = "WIP.")]
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

    let (events_tx, events_rx) = mpsc::unbounded_channel();

    let mut state = State::new(events_rx);

    let input_handler = task::spawn(handle_input(events_tx));
    let renderer = task::spawn(render(parsed_constants, state));

    match future::try_join(input_handler, renderer).await? {
        (Err(err), _) | (_, Err(err)) => Err(err),
        _ => Ok(()),
    }
}
