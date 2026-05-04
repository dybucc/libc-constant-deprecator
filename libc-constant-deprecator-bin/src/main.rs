#![expect(unused, reason = "WIP.")]

use std::{
    env,
    io::{self, Stdout, StdoutLock, Write},
    ops::{Deref, Range},
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

#[derive(Debug)]
pub(crate) struct Termination<T> {
    repr: TerminationRepr<T>,
}

impl<T> Termination<T> {
    pub(crate) fn should_terminate(&self) -> bool {
        matches!(self.repr, TerminationRepr::Termination)
    }

    pub(crate) fn keep_going(val: T) -> Self {
        Self {
            repr: TerminationRepr::NonTermination(val),
        }
    }

    pub(crate) fn terminate() -> Self {
        Self {
            repr: TerminationRepr::Termination,
        }
    }

    pub(crate) fn into_inner(self) -> T {
        self.repr.unwrap()
    }
}

#[derive(Debug)]
pub(crate) enum TerminationRepr<T> {
    Termination,
    NonTermination(T),
}

impl<T> TerminationRepr<T> {
    #[track_caller]
    pub(crate) fn unwrap(self) -> T {
        match self {
            Self::Termination => panic!(),
            Self::NonTermination(val) => val,
        }
    }
}

#[derive(Debug)]
pub(crate) struct State {
    events: UnboundedReceiver<RawUserEvent>,
    mode: Mode,
    filter_buf: BorrowedContainer,
    // The logic here is that even if in insert mode, the last selected constant is meant to be the
    // one that was under the cursor prior to entering insert mode. In normal mode, it is the last
    // registered constant that was under the cursor, and in select mode it is a range that could
    // comprise a set of values larger than 1.
    selected: Range<usize>,
    prompt: String,
}

// NOTE: we will require modifying some of the routines here once we get to
// implementing scrolling in the 10-row constant symbol layout.
impl State {
    pub(crate) fn new(borrowed_view: BorrowedContainer) -> (Self, UnboundedSender<RawUserEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();

        (
            Self {
                events: rx,
                mode: Mode::default(),
                filter_buf: borrowed_view,
                selected: Range::default(),
                prompt: String::default(),
            },
            tx,
        )
    }

    pub(crate) fn update(&mut self, event: Option<UserEvent>) {
        let Self {
            filter_buf,
            selected,
            prompt,
            ..
        } = self;

        let Some(event) = event else {
            return;
        };

        // TODO: actually get to updating the stuff.

        todo!()
    }

    pub(crate) fn receive_event(&mut self) -> Termination<Option<UserEvent>> {
        let Self { events, mode, .. } = self;

        match events.try_recv() {
            Ok(event) if let Some(event) = mode.interpret(&event) => {
                Termination::keep_going(event.into())
            }
            Err(TryRecvError::Disconnected) => Termination::terminate(),
            _ => Termination::keep_going(None),
        }
    }

    pub(crate) fn draw(&self, mut stdout: impl Write) -> anyhow::Result<()> {
        let Self {
            events,
            mode,
            filter_buf,
            selected,
            prompt,
        } = self;

        crossterm::queue!(stdout, Print("> "))?;

        // TODO: finish drawing the ten-row table with the constant symbols, those which
        // are currently selected if inside select mode, and those marked deprecated.
        // The mark for deprecation is something specific to each constant that is
        // already kept track of within the borrowed view into the set of constants. The
        // selection status is kept track of by the instance of `State` that is handling
        // this drawing call.

        stdout.flush().map_err(Into::into)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Mode {
    repr: ModeRepr,
}

impl Mode {
    fn new(repr: ModeRepr) -> Self {
        Self { repr }
    }

    pub(crate) fn kind(&self) -> ModeRepr {
        self.repr
    }

    pub(crate) fn is_normal(&self) -> bool {
        matches!(self.repr, ModeRepr::Normal)
    }

    pub(crate) fn is_insert(&self) -> bool {
        matches!(self.repr, ModeRepr::Insert)
    }

    pub(crate) fn is_select(&self) -> bool {
        matches!(self.repr, ModeRepr::Select)
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

    pub(crate) fn interpret(&self, event: &RawUserEvent) -> Option<UserEvent> {
        match (self.repr, event.inner()) {
            // TODO: it may prove beneficial to further filter out plain text input, as there may be
            // some characters that just shouldn't even be printable on-screen.
            (ModeRepr::Insert, RawUserEventRepr::PlainText(c)) => UserEvent::text(c).into(),
            (ModeRepr::Insert, RawUserEventRepr::Space) => UserEvent::text(' ').into(),
            (ModeRepr::Normal, RawUserEventRepr::PlainText(c))
                if let Some(action) = ModeAction::is_navigation(c) =>
            {
                UserEvent::action(action).into()
            }
            (ModeRepr::Normal, RawUserEventRepr::PlainText(c))
                if let Some(action) = ModeAction::is_to_mode(c, Mode::insert()) =>
            {
                UserEvent::action(action).into()
            }
            (ModeRepr::Normal, RawUserEventRepr::PlainText(c))
                if let Some(action) = ModeAction::is_to_mode(c, Mode::select()) =>
            {
                UserEvent::action(action).into()
            }
            (ModeRepr::Normal, RawUserEventRepr::Return) => UserEvent::search().into(),
            (ModeRepr::Normal, RawUserEventRepr::ShiftReturn) => UserEvent::effect().into(),
            (ModeRepr::Normal, RawUserEventRepr::Escape) => UserEvent::clear().into(),
            (ModeRepr::Select, RawUserEventRepr::PlainText(c))
                if let Some(action) = ModeAction::is_navigation(c) =>
            {
                UserEvent::action(action).into()
            }

            // Toggling the deprecated state of a set of contant symbols can happen in both normal
            // mode and in select mode. The former performs deprecation of the constant symbol under
            // the cursor, while the latter does so for a range of symbols under the cursor (which
            // could be 1+.)
            (ModeRepr::Normal | ModeRepr::Select, RawUserEventRepr::Space) => {
                UserEvent::toggle().into()
            }

            // If in any one of insert or select mode, just move back to normal mode.
            (ModeRepr::Insert | ModeRepr::Select, RawUserEventRepr::Escape) => {
                UserEvent::action(ModeAction::switch_modes(Mode::normal())).into()
            }

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
    fn new(event: UserEventRepr) -> Self {
        Self { repr: event }
    }

    pub(crate) fn is_text(&self) -> Option<char> {
        if let UserEventRepr::TextualInput(c) = self.repr {
            c.into()
        } else {
            None
        }
    }

    pub(crate) fn is_search(&self) -> bool {
        matches!(self.repr, UserEventRepr::Search)
    }

    pub(crate) fn is_toggle(&self) -> bool {
        matches!(self.repr, UserEventRepr::Toggle)
    }

    pub(crate) fn is_effect(&self) -> bool {
        matches!(self.repr, UserEventRepr::Effect)
    }

    pub(crate) fn is_clear(&self) -> bool {
        matches!(self.repr, UserEventRepr::Clear)
    }

    pub(crate) fn is_action(&self) -> Option<&ModeAction> {
        if let UserEventRepr::ModeAction(action) = &self.repr {
            action.into()
        } else {
            None
        }
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

impl ModeAction {
    fn new(repr: ModeActionRepr) -> Self {
        Self { repr }
    }

    pub(crate) fn switch_modes(new_mode: Mode) -> Self {
        Self {
            repr: ModeActionRepr::ModeSwitch(new_mode),
        }
    }

    pub(crate) fn is_to_mode(c: char, mode: Mode) -> Option<Self> {
        if c == 'i' && mode.is_insert() || c == 'v' && mode.is_select() {
            ModeAction::switch_modes(mode).into()
        } else {
            None
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
    fn new(event: RawUserEventRepr) -> Self {
        Self { repr: event }
    }

    pub(crate) fn inner(&self) -> RawUserEventRepr {
        self.repr
    }

    pub(crate) fn space() -> Self {
        Self {
            repr: RawUserEventRepr::Space,
        }
    }

    pub(crate) fn text(c: char) -> Self {
        Self {
            repr: RawUserEventRepr::PlainText(c),
        }
    }

    pub(crate) fn ret() -> Self {
        Self {
            repr: RawUserEventRepr::Return,
        }
    }

    pub(crate) fn sret() -> Self {
        Self {
            repr: RawUserEventRepr::ShiftReturn,
        }
    }

    pub(crate) fn esc() -> Self {
        Self {
            repr: RawUserEventRepr::Escape,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RawUserEventRepr {
    PlainText(char),
    Return,
    // NOTE: even though this could be passed as plain text input, we prefer to keep it as a
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
        // NOTE: there may be a better way of doing this, as I don't believe constant
        // moves on every loop iteration are even remotely optimal.
        (state, stdout) = task::spawn_blocking(move || draw_screen(state, stdout)).await??;

        if update(&mut state).should_terminate() {
            break Ok(());
        }
    }
}

pub(crate) fn draw_screen(
    state: State,
    mut stdout: MutexGuard<'_, Stdout>,
) -> anyhow::Result<(State, MutexGuard<'_, Stdout>)> {
    state.draw(&mut *stdout)?;

    Ok((state, stdout))
}

pub(crate) fn update(state: &mut State) -> Termination<()> {
    let res = state.receive_event();

    if res.should_terminate() {
        return Termination::terminate();
    }

    // NOTE: this won't panic because the inner representation for a state of
    // termination has already been proved to not be a termination state.
    state.update(res.into_inner());

    todo!()
}

pub(crate) async fn handle_input(channel: UnboundedSender<RawUserEvent>) -> anyhow::Result<()> {
    let mut event_stream = EventStream::new().fuse();

    while let Some(event) = event_stream.next().await {
        match event? {
            Event::Key(KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::space()),
            Event::Key(KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::text(c)),
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::ret()),
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::SHIFT,
                ..
            }) => _ = channel.send(RawUserEvent::sret()),
            Event::Key(KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::esc()),

            // The termination event, which should break out of the loop and drop the producer end
            // of the channel to have the receiver end indicate termination to the task managing it.
            Event::Key(KeyEvent {
                code: KeyCode::Char('c' | 'd'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }) => break,

            _ => (),
        }
    }

    drop(channel);

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
