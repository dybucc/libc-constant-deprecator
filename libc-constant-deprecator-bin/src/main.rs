#![expect(unused, reason = "WIP.")]

use std::{
    env,
    io::{self, Stdout, StdoutLock, Write},
    ops::Deref,
    path::PathBuf,
    sync::LazyLock,
};

use clap::Parser;
use crossterm::{
    cursor::{self, MoveToNextLine, MoveToRow, SetCursorStyle},
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
    style::Print,
    terminal::{self, Clear, ClearType, DisableLineWrap},
};
use futures::{StreamExt, future};
use libc_constant_deprecator_lib::{BorrowedContainer, ConstContainer};
use tokio::{
    process::ChildStdout,
    sync::{
        Mutex, MutexGuard,
        mpsc::{self, UnboundedReceiver, UnboundedSender, error::TryRecvError},
    },
    task,
};

#[derive(Debug, Parser)]
pub(crate) struct Args {
    path: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Termination {
    repr: TerminationRepr,
}

impl Termination {
    pub(crate) fn should_terminate(&self) -> bool {
        matches!(self.repr, TerminationRepr::Termination)
    }

    pub(crate) fn terminate() -> Self {
        Self {
            repr: TerminationRepr::Termination,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) enum TerminationRepr {
    Termination,
    #[default]
    NonTermination,
}

#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct State {
    events: UnboundedReceiver<RawUserEvent>,
    mode: Mode,
    filter_buf: BorrowedContainer,
}

impl State {
    pub(crate) fn new(borrowed_view: BorrowedContainer) -> (Self, UnboundedSender<RawUserEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();

        (
            Self {
                events: rx,
                mode: Mode::default(),
                filter_buf: borrowed_view,
            },
            tx,
        )
    }

    pub(crate) fn receive_event(&mut self) -> Termination {
        let Self { events, mode, .. } = self;
        let event = match events.try_recv() {
            Ok(event) => event,
            Err(TryRecvError::Empty) => return Termination::default(),
            Err(_) => return Termination::terminate(),
        };
        let res = mode.meaning(&event);

        Termination::default()
    }

    pub(crate) fn draw(&self, mut stdout: impl Write) -> anyhow::Result<()> {
        let State {
            events,
            mode,
            filter_buf,
        } = self;

        crossterm::queue!(stdout, Print("> "))?;

        stdout.flush().map_err(Into::into)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Mode {
    repr: ModeRepr,
}

impl Mode {
    pub(crate) fn new(repr: ModeRepr) -> Self {
        Self { repr }
    }

    pub(crate) fn kind(&self) -> ModeRepr {
        self.repr
    }

    pub(crate) fn normal() -> Self {
        Self {
            repr: ModeRepr::Normal,
        }
    }

    pub(crate) fn insert() -> Self {
        Self {
            repr: ModeRepr::Insert,
        }
    }

    pub(crate) fn select() -> Self {
        Self {
            repr: ModeRepr::Select,
        }
    }

    // NOTE: we return an `Option` here because there's some events which, within
    // certain modes, just don't have any meaning once interpreted beyond raw user
    // input.
    pub(crate) fn meaning(&self, event: &RawUserEvent) -> Option<UserEvent> {
        match (self.repr, event.kind()) {
            // TODO: it may prove beneficial to further filter out plain text input, as there may be
            // some keys that just shouldn't even be printable on-screen.
            (ModeRepr::Insert, RawUserEventRepr::PlainText(c)) => UserEvent::text(c).into(),
            (ModeRepr::Insert, RawUserEventRepr::Space) => UserEvent::text(' ').into(),
            (ModeRepr::Insert, RawUserEventRepr::Escape) => UserEvent::action(ModeAction::new(
                ModeActionRepr::ModeSwitch(Mode::new(ModeRepr::Normal)),
            ))
            .into(),
            (ModeRepr::Normal, RawUserEventRepr::PlainText(c))
                if let Some(action) = ModeAction::is_navigation(c) =>
            {
                UserEvent::new(UserEventRepr::ModeAction(action)).into()
            }
            (ModeRepr::Normal, RawUserEventRepr::Return) => todo!(),
            (ModeRepr::Normal, RawUserEventRepr::Space) => todo!(),
            (ModeRepr::Normal, RawUserEventRepr::ShiftReturn) => todo!(),
            (ModeRepr::Normal, RawUserEventRepr::Escape) => todo!(),
            (ModeRepr::Select, RawUserEventRepr::PlainText(_)) => todo!(),
            (ModeRepr::Select, RawUserEventRepr::Return) => todo!(),
            (ModeRepr::Select, RawUserEventRepr::Space) => todo!(),
            (ModeRepr::Select, RawUserEventRepr::ShiftReturn) => todo!(),
            (ModeRepr::Select, RawUserEventRepr::Escape) => todo!(),

            // NOTE: this includes a bunch of cases where input processing shouldn't even be
            // performed, as the event that has taken place has no meaning in the currently active
            // mode.
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) enum ModeRepr {
    Insert,
    #[default]
    Normal,
    Select,
}

#[derive(Debug)]
pub(crate) struct UserEvent {
    repr: UserEventRepr,
}

impl UserEvent {
    pub(crate) fn new(event: UserEventRepr) -> Self {
        Self { repr: event }
    }

    pub(crate) fn text(c: char) -> Self {
        Self {
            repr: UserEventRepr::TextualInput(c),
        }
    }

    pub(crate) fn search() -> Self {
        Self {
            repr: UserEventRepr::Search,
        }
    }

    pub(crate) fn toggle() -> Self {
        Self {
            repr: UserEventRepr::Toggle,
        }
    }

    pub(crate) fn effect() -> Self {
        Self {
            repr: UserEventRepr::Effect,
        }
    }

    pub(crate) fn clear() -> Self {
        Self {
            repr: UserEventRepr::Clear,
        }
    }

    pub(crate) fn action(action: ModeAction) -> Self {
        Self {
            repr: UserEventRepr::ModeAction(action),
        }
    }
}

#[derive(Debug)]
enum UserEventRepr {
    // Corresponds with plain text user input at the prompt.
    TextualInput(char),
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
    // Is triggered when going from insert mode to normal mode.
    ModeAction(ModeAction),
}

// NOTE: this type serves as a general LUT for all commands involving anything
// but (1) major actions, which are stored inline under `UserEvent`, and (2)
// plain text user input.
#[derive(Debug)]
pub(crate) struct ModeAction {
    repr: ModeActionRepr,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub(crate) fn up() -> Self {
        Self::Up
    }

    pub(crate) fn down() -> Self {
        Self::Down
    }

    pub(crate) fn left() -> Self {
        Self::Left
    }

    pub(crate) fn right() -> Self {
        Self::Right
    }
}

impl ModeAction {
    pub(crate) fn new(repr: ModeActionRepr) -> Self {
        Self { repr }
    }

    pub(crate) fn switch_modes(new_mode: Mode) -> Self {
        Self {
            repr: ModeActionRepr::ModeSwitch(new_mode),
        }
    }

    pub(crate) fn navigate(dir: Dir) -> Self {
        match dir {
            Dir::Up => Self {
                repr: ModeActionRepr::GoUp,
            },
            Dir::Down => Self {
                repr: ModeActionRepr::GoDown,
            },
            Dir::Left => Self {
                repr: ModeActionRepr::GoLeft,
            },
            Dir::Right => Self {
                repr: ModeActionRepr::GoRight,
            },
        }
    }

    pub(crate) fn is_navigation(c: char) -> Option<Self> {
        match c {
            'h' => Self::new(ModeActionRepr::GoLeft).into(),
            'l' => Self::new(ModeActionRepr::GoRight).into(),
            'k' => Self::new(ModeActionRepr::GoUp).into(),
            'j' => Self::new(ModeActionRepr::GoDown).into(),
            _ => None,
        }
    }
}

#[derive(Debug)]
enum ModeActionRepr {
    // Corresponds with using the escape key in insert mode to go back to normal mode.
    ModeSwitch(Mode),
    // Corresponds with the navigation binding assigned to `h`.
    GoLeft,
    // Corresponds with the navigation binding assigned to `l`.
    GoRight,
    // Corresponds with the navigation binding assigned to `k`.
    GoUp,
    // Corresponds with the navigation binding assigned to `j`.
    GoDown,
}

#[derive(Debug)]
pub(crate) struct RawUserEvent {
    repr: RawUserEventRepr,
}

impl RawUserEvent {
    pub(crate) fn kind(&self) -> RawUserEventRepr {
        self.repr
    }

    pub(crate) fn new(event: RawUserEventRepr) -> Self {
        Self { repr: event }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RawUserEventRepr {
    PlainText(char),
    Return,
    // NOTE: even though this could pass off as character input, we prefer to keep it as a
    // separate variant for the purposes of easing other user commands once they're processed
    // beyond raw user events.
    Space,
    ShiftReturn,
    Escape,
}

pub(crate) static SYNC_BUF: LazyLock<Mutex<Stdout>> = LazyLock::new(|| Mutex::new(io::stdout()));

pub(crate) async fn render(mut constants: ConstContainer, mut state: State) -> anyhow::Result<()> {
    let mut stdout = SYNC_BUF.lock().await;

    loop {
        (state, stdout) = task::spawn_blocking(move || draw_screen(state, stdout)).await??;
        update(&mut state);
    }
}

pub(crate) fn draw_screen(
    state: State,
    mut stdout: MutexGuard<'_, Stdout>,
) -> anyhow::Result<(State, MutexGuard<'_, Stdout>)> {
    state.draw(&mut *stdout)?;

    Ok((state, stdout))
}

pub(crate) fn update(state: &mut State) -> Termination {
    if state.receive_event().should_terminate() {
        Termination::terminate()
    } else {
        Termination::default()
    }
}

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

pub(crate) async fn prepare_space() -> anyhow::Result<()> {
    let (rows, _) = terminal::size()?;
    let (current_row, _) = cursor::position()?;

    let mut stdout = SYNC_BUF.lock().await;

    // NOTE: the initial cursor is meant to reflect the shape of normal mode, which
    // should allow navigation throughout both the set of constants on display and
    // the prompt.
    crossterm::queue!(stdout, SetCursorStyle::SteadyBlock, DisableLineWrap)?;

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
    let (state, events_tx) = State::new(parsed_constants.borrowed());

    prepare_space().await?;

    let input_handler = task::spawn(handle_input(events_tx));
    let renderer = task::spawn(render(parsed_constants, state));

    future::try_join(input_handler, renderer)
        .await
        .map(|(res1, res2)| res1.and(res2))?
}
