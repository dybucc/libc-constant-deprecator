#![expect(unused, reason = "WIP.")]
#![feature(range_bounds_is_empty)]

use std::{
    env,
    io::{self, Stdout, StdoutLock, Write},
    ops::{Add, AddAssign, ControlFlow, Deref, Range, RangeBounds, Sub, SubAssign},
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
use libc_constant_deprecator_lib::{BorrowedContainer, Const, ConstContainer, Visit};
use tokio::{
    process::ChildStdout,
    sync::{
        Mutex, MutexGuard,
        mpsc::{self, UnboundedReceiver, UnboundedSender, error::TryRecvError},
    },
    task,
};

// NOTE: this is used for the purposes of easing declaration of two sets of
// enums from the same variants. This then allows to implement internal
// functionality in terms of the fields without public values, and internal
// functionality in terms of variants with compound fields. It also defines an
// impl with a method to transition between the internal, field-rich
// representation to the public-facing external representation, which loses all
// of the field information for the purposes of opaqueness.
// NOTE: we use `allow` here instead of the usual `expect` because the lint
// system seems to be getting wrong the `unfulflled_lint_expectation` lint when
// using the latter.
macro_rules! repr {
    // NOTE: we don't take into consideration parametric polymorphism for the
    // public type that is generated because that enumeration is not meant to
    // hold any real information beyond the opaque representation of the
    // internal type. We also force the presence of at least one attribute to
    // enforce the implementation of `Debug` on all types.
    (
        $(#[$pub_attr:meta])+
        $pub:tt // The public-facing, opaque representation.
        $(#[$priv_attr:meta])+
        $priv:tt$(<$($t:tt),+>)? => // The internal, field-rich type.
        {
            $(
                $($(#[$var_attr:meta])+)? // Matches attributes like `#[default]`.
                $var:ident // Matches the actual identifier for the variant.
                $(($($content:tt),*))? // Matches a tuple variant.
                $({$($field_content:tt: $ty_content:ty),*})? // Matches a struct-like variant.
            ), +$(,)?
        }
        $(#[$wrap_attr:meta])+
        $wrap:tt // The wrapper type around the internal representation.
        // Which may or may not have fields other than the internal `repr` we generate it with.
        $({
            $($wrap_field:tt: $wrap_type:ty),+
        })?
    ) => {
        // NOTE: the treatment of field variants is not exhaustive, and it could
        // cause errors if the provided syntax tree in the matched macro subtree
        // is not valid Rust code. We assume such errors would be easy to spot
        // at invocation site.
        $(#[$priv_attr])*
        enum $priv$(<$($t),+>)? {
            $(
                $($(#[$var_attr])+)? $var$(($($content),*))?$({$($field_content: $ty_content),*})?,
            )+
        }

        // NOTE: the impl for the internal representation is verbosily repeated for both the public
        // type implementing the kind and the internal type denoting the variants because its body
        // requires metavariable expansion at multiple levels of depth.
        impl$(<$($t),+>)? $priv$(<$($t),+>)? {
            fn map_public(&self) -> $pub {
                #[allow(
                    non_snake_case,
                    reason = "The macro that produces this method cannot match against anything \
                              but the metavariables that repeat at the required depth, so the \
                              bindings in the below pattern have the same identifiers as the types \
                              of the corresponding fields in their declaration (i.e. uppercase.)"
                )]

                match self {
                    $(
                        Self::$var$(($($content),*))?$({$(_$field_content),*})? => $pub::$var
                    ),+
                }
            }
        }

        $(#[$pub_attr])*
        pub(crate) enum $pub {
            $(
                $($(#[$var_attr])+)? $var,
            )+
        }

        $(#[$wrap_attr])*
        pub(crate) struct $wrap$(<$($t),+>)? {
            repr: $priv$(<$($t),+>)?,
            $($($wrap_field: $wrap_type),+)?
        }

        impl$(<$($t),+>)? $wrap$(<$($t),+>)? {
            fn kind(&self) -> $pub {
                self.repr.map_public()
            }
        }
    };
}

// NOTE: this abstracts over the implementation of checker methods expressed in
// terms of the internal representation of a given type, and over the
// implementation of both constructors and infallible-unwrappers that panic if
// the underlying representation is not the requested one. This is especially
// suited for types generated using the above macro -- `repr`.
//
// The implementation requires naming tuple fields for the purposes of having a
// metavariable to match for routine parameters, which unlike pattern
// destructuring, cannot accept type identifiers.
macro_rules! trivial_impl {
    ($over_t:tt => {
        $(
            $ctor:ident, // The constructor that yields the wrapper type over the internal repr.
            $fallible_is:ident, // The checker operation to ensure a certain variant is within it.
            $infallible_is:ident, // The infallible operation that unwraps a variant or panics.
            $infallible_is_ref:ident // The non-consuming version of the infallible op.
            => $var:tt $(($($tuple:tt: $tuple_t:ty),*))? $({$($field:tt: $field_t:ty),*})?
        );+ ;
    }) => {
        $(
            pub(crate) fn $ctor($($($tuple: $tuple_t),*)? $($($field: $field_t),*)?) -> Self {
                Self { repr: $over_t::$var $(($($tuple),*))? $({$($field),*})? }
            }

            $(
                #[track_caller]
                pub(crate) fn $infallible_is(self) -> ($($tuple_t),*) {
                    if let $over_t::$var(res) = self.repr {
                        res
                    } else {
                        let mut err = String::new();
                        $(err.push_str(stringify!($tuple_t));)*
                        panic!("variant did not contain {}", err);
                    }
                }

                #[track_caller]
                pub(crate) fn $infallible_is_ref(&self) -> ($(&$tuple_t),*) {
                    if let $over_t::$var(res) = &self.repr {
                        res
                    } else {
                        let mut err = String::new();
                        $(err.push_str(stringify!($tuple_t));)*
                        panic!("variant did not contain {}", err);
                    }
                }
            )?

            $(
                #[track_caller]
                pub(crate) fn $infallible_is(self) -> ($($field),*) {
                    if let $over_t::$var{$($field:tt),*} = self.repr {
                        ($($field),*)
                    } else {
                        let mut err = String::new();
                        $(err.push_str(stringify!($field_t));)*
                        panic!("variant did not contain {}", err);
                    }
                }

                #[track_caller]
                pub(crate) fn $infallible_is_ref(self) -> ($(&$field),*) {
                    if let $over_t::$var{$($field:tt),*} = &self.repr {
                        ($($field),*)
                    } else {
                        let mut err = String::new();
                        $(err.push_str(stringify!($field_t));)*
                        panic!("variant did not contain {}", err);
                    }
                }
            )?

            pub(crate) fn $fallible_is(&self) -> bool {
                matches!(&self.repr, $over_t::$var $(($($tuple),*))? $({$($field),*})?)
            }
        )+
    };
}

#[derive(Debug, Parser)]
pub(crate) struct Args {
    path: Option<String>,
}

repr! {
    #[derive(Debug)]
    TerminationKind
    #[derive(Debug)]
    TerminationRepr<T> => {
        Termination,
        NonTermination(T)
    }
    #[derive(Debug)]
    Termination
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

    #[track_caller]
    pub(crate) fn into_inner(self) -> T {
        self.repr.unwrap()
    }
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
    constants: ConstContainer,
    filter_buf: BorrowedContainer,
    // NOTE: the following invariant holds for the range of selected symbols:
    // - If in normal mode, the range is empty. This, by extension, also holds when entering insert
    //   mode though that matters not because the only actions available in insert mode are those
    //   for switching modes and those for searching.
    // - If in select mode, the range may be empty or not. The logic here is that it starts off
    //   empty as it comes from normal mode, but can be extended within select mode. A consequence
    //   of this is that range selection is reset back to an empty range once the user exits out of
    //   select mode.
    selected: Range<usize>,
    prompt: String,
    pos: Position,
}

// NOTE: we will require modifying some of the routines here once we get to
// implementing scrolling in the 10-row list of constant symbols.
impl State {
    pub(crate) fn new(constants: ConstContainer) -> (Self, UnboundedSender<RawUserEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();

        // NOTE: it could be that the `Default` impl of `Range` does not produce an
        // empty range (it should, though, because it relies on the `Default` impl of
        // the `Idx` polymorphic parameter; A `usize` here,) so keep an eye out for that
        // once testing starts.
        (
            Self {
                events: rx,
                mode: Mode::default(),
                filter_buf: constants.borrowed(),
                constants,
                selected: Range::default(),
                prompt: String::default(),
                pos: Position::default(),
            },
            tx,
        )
    }

    pub(crate) async fn update(&mut self, event: Option<UserEvent>) -> anyhow::Result<()> {
        let Self {
            filter_buf,
            selected,
            prompt,
            constants,
            mode,
            pos,
            ..
        } = self;

        let Some(event) = event else {
            return Ok(());
        };

        match event.kind() {
            // NOTE: this is not quite as ergonomic as getting the event payload directly from the
            // internal representation, but it gets the job done just fine.
            UserEventKind::TextualInput => prompt.push(event.text()),
            UserEventKind::Search => constants.filter_with(&prompt, filter_buf)?,
            // NOTE: toggles all constants because we are either (1) outside select mode, or (2)
            // within it but without a selection ranging beyond a single row.
            UserEventKind::Toggle if <Range<usize> as RangeBounds<usize>>::is_empty(selected) => {
                if all_deprecated(filter_buf) {
                    filter_buf.undeprecate();
                } else {
                    filter_buf.deprecate();
                }
            }
            // NOTE: this uses `BorrowedSubset` to add another degree of refinement to the set of
            // constants currently under consideration. This only happens in select mode, where the
            // range in `selected` is non-empty.
            UserEventKind::Toggle => {
                let mut selected = filter_buf.select(selected);

                if all_deprecated(&selected) {
                    selected.undeprecate();
                } else {
                    selected.deprecate();
                }
            }
            UserEventKind::Effect => constants.effect_changes().await?,
            UserEventKind::Clear => constants.filter_with(".*", filter_buf)?,
            UserEventKind::ModeAction => {
                let action = event.action();

                match action.kind() {
                    ModeActionKind::ModeSwitch => *mode = action.mode(),
                    // TODO: use the utilities implemented for the `Position` type to more
                    // ergonomically implement navigation. Take into account the fact the user could
                    // be in both normal and select mode when a navigation event is triggered.
                    ModeActionKind::GoLeft => todo!(),
                    ModeActionKind::GoRight => todo!(),
                    ModeActionKind::GoUp => todo!(),
                    ModeActionKind::GoDown => todo!(),
                }
            }
        }

        Ok(())
    }

    pub(crate) fn receive_event(&mut self) -> Termination<Option<UserEvent>> {
        let Self { events, mode, .. } = self;

        match events.try_recv() {
            Ok(event) => mode
                .interpret(event)
                .map_or_else(Termination::terminate, |event| {
                    Termination::keep_going(event.into())
                }),
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
            ..
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

fn all_deprecated(visited: &impl Visit) -> bool {
    let mut check = true;

    visited.visit(|constant| {
        if !constant.is_deprecated() {
            check = false;
            return ControlFlow::Break(());
        }

        ControlFlow::Continue(())
    });

    check
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Position {
    row: u16,
    col: u16,
}

impl SubAssign for Position {
    fn sub_assign(&mut self, rhs: Self) {
        let Self { row, col } = self;
        let Self {
            row: orow,
            col: ocol,
        } = rhs;

        *row = row.saturating_sub(orow);
        *col = col.saturating_sub(ocol);
    }
}

impl Sub for Position {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        let Self { row, col } = self;
        let Self {
            row: orow,
            col: ocol,
        } = rhs;

        Self {
            row: row.saturating_sub(orow),
            col: col.saturating_sub(ocol),
        }
    }
}

impl AddAssign for Position {
    fn add_assign(&mut self, rhs: Self) {
        let Self { row, col } = self;
        let Self {
            row: orow,
            col: ocol,
        } = rhs;
        let (max_row, max_col) = terminal::size().unwrap();

        if *row + orow < max_row && *col + ocol < max_col {
            *row += orow;
            *col += ocol;
        } else {
            *row = max_row - 1;
            *col = max_col - 1;
        }
    }
}

impl Add for Position {
    type Output = Self;

    /// Because the `Saturating` type in `std::num` does not seem to be usable
    /// with non-primitive types, this operation performs saturating arithmetic,
    /// not based off of the numerical limits but rather off of the terminal
    /// dimensions.
    fn add(self, rhs: Self) -> Self::Output {
        let Self { row, col } = self;
        let Self {
            row: orow,
            col: ocol,
        } = rhs;
        let (max_row, max_col) = terminal::size().unwrap();

        // NOTE: here both `row` and `orow` are 0-indexed, but `max_row` is 1-indexed,
        // so we must compare for a relation of strict "less than," and in the `else`
        // branch perform unit subtraction.
        if row + orow < max_row && col + ocol < max_col {
            Self {
                row: row + orow,
                col: col + ocol,
            }
        } else {
            Self {
                row: max_row - 1,
                col: max_col - 1,
            }
        }
    }
}

impl Default for Position {
    fn default() -> Self {
        // NOTE: this is only called when initializing the running state, which itself
        // only happens right after calling `prepare_space()`. If that routine did not
        // fail (and it shouldn't have if we've reached this point,) then the only
        // chance this fails is if some unknown I/O-bound error takes place.
        let (row, col) = cursor::position().unwrap();

        Self { row, col }
    }
}

repr! {
    #[derive(Debug, Default, Clone, Copy)]
    ModeKind
    #[derive(Debug, Default, Clone, Copy)]
    ModeRepr => {
        Insert,
        #[default]
        Normal,
        Select,
    }
    #[derive(Debug, Default)]
    Mode
}

impl Mode {
    fn new(repr: ModeRepr) -> Self {
        Self { repr }
    }

    pub(crate) fn interpret(&self, event: RawUserEvent) -> Option<UserEvent> {
        match (self.kind(), event.kind()) {
            // Insert mode
            (ModeKind::Insert, RawUserEventKind::PlainText) => UserEvent::new_text(event.text()),
            (ModeKind::Insert, RawUserEventKind::Space) => UserEvent::new_text(' '),

            // Normal mode.
            (ModeKind::Normal, RawUserEventKind::PlainText) => {
                let text = event.text();

                // NOTE: we have to keep this logic mixed because otherwise the payload is lost
                // when moved out of `event` (by the `text()` method.)
                if let Some(action) = ModeAction::is_navigation(text) {
                    UserEvent::new_action(action)
                } else {
                    UserEvent::new_action(ModeAction::is_mode_transition(text)?)
                }
            }
            (ModeKind::Normal, RawUserEventKind::ShiftReturn) => UserEvent::new_effect(),
            (ModeKind::Normal, RawUserEventKind::Escape) => UserEvent::new_clear(),

            // Select mode.
            (ModeKind::Select, RawUserEventKind::PlainText)
                if let Some(action) = ModeAction::is_navigation(event.text()) =>
            {
                UserEvent::new_action(action)
            }

            // Shared cases.
            (ModeKind::Insert | ModeKind::Normal, RawUserEventKind::Return) => {
                UserEvent::new_search()
            }
            (ModeKind::Normal | ModeKind::Select, RawUserEventKind::Space) => {
                UserEvent::new_toggle()
            }
            (ModeKind::Insert | ModeKind::Select, RawUserEventKind::Escape) => {
                UserEvent::new_action(ModeAction::switch_modes(Mode::new_normal()))
            }

            // NOTE: this includes a bunch of cases where input processing shouldn't even be
            // performed, as the combination of mode/event does not make for a viable event to
            // process.
            _ => return None,
        }
        .into()
    }

    trivial_impl! { ModeRepr => {
        new_normal, is_normal, normal, normal_ref => Normal;
        new_insert, is_insert, insert, insert_ref => Insert;
        new_select, is_select, select, select_ref => Select;
    }}
}

repr! {
    #[derive(Debug)]
    UserEventKind
    #[derive(Debug)]
    UserEventRepr => {
        // Corresponds with plain text user input at the prompt.
        TextualInput(char),
        // Is triggered with the return key and should trigger a filtering event with the current
        // contents of the prompt.
        Search,
        // Is triggered with the space key and should toggle all selected constants' state to
        // "deprecated", unless all selected constants are already deprecated, in which case it
        // should undeprecate them.
        Toggle,
        // Is triggered with the shift + return combo and should effect the changes to disk.
        Effect,
        // Is triggered with the escape key and should clear the currently input regex.
        Clear,
        // Is triggered when going from insert mode to normal mode.
        ModeAction(ModeAction),
    }
    #[derive(Debug)]
    UserEvent
}

impl UserEvent {
    fn new(event: UserEventRepr) -> Self {
        Self { repr: event }
    }

    trivial_impl! { UserEventRepr => {
        new_text, is_text, text, text_ref => TextualInput(c: char);
        new_action, is_action, action, action_ref => ModeAction(action: ModeAction);
        new_search, is_search, search, search_ref => Search;
        new_toggle, is_toggle, toggle, toggle_ref => Toggle;
        new_effect, is_effect, effect, effect_ref => Effect;
        new_clear, is_clear, clear, clear_ref => Clear;
    }}
}

// NOTE: this type serves as a general LUT for all commands involving anything
// but (1) major actions, which are stored inline under `UserEvent`, and (2)
// plain text user input.
repr! {
    #[derive(Debug)]
    ModeActionKind
    #[derive(Debug)]
    ModeActionRepr => {
        // Corresponds with using the escape key in insert mode to go back to normal mode.
        ModeSwitch(Mode),
        // Corresponds with the navigation binding assigned to `h`.
        GoLeft,
        // Corresponds with the navigation binding assigned to `l`.
        GoRight,
        // Corresponds with the navigation binding assigned to `k`.
        GoUp,
        // Corresponds with the navigation binding assigned to `j`.
        GoDown
    }
    #[derive(Debug)]
    ModeAction
}

impl ModeAction {
    fn new(repr: ModeActionRepr) -> Self {
        Self { repr }
    }

    trivial_impl! { ModeActionRepr => {
        switch_modes, is_mode, mode, mode_ref => ModeSwitch(mode: Mode);
        new_left, is_left, left, left_ref => GoLeft;
        new_right, is_right, right, right_ref => GoRight;
        new_up, is_up, up, up_ref => GoUp;
        new_down, is_down, down, down_ref => GoDown;
    }}

    pub(crate) fn is_mode_transition(c: char) -> Option<Self> {
        match c {
            'i' => ModeAction::switch_modes(Mode::new_insert()),
            'v' => ModeAction::switch_modes(Mode::new_select()),
            _ => return None,
        }
        .into()
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

repr! {
    #[derive(Debug, Clone, Copy)]
    RawUserEventKind
    #[derive(Debug, Clone, Copy)]
    RawUserEventRepr => {
        PlainText(char),
        Return,
        // NOTE: even though this could be passed as plain text input, we prefer to keep it as a
        // separate variant for the purposes of easing other user commands once they're processed
        // beyond raw user events.
        Space,
        ShiftReturn,
        Escape,
    }
    #[derive(Debug)]
    RawUserEvent
}

impl RawUserEvent {
    fn new(event: RawUserEventRepr) -> Self {
        Self { repr: event }
    }

    trivial_impl! { RawUserEventRepr => {
        new_space, is_space, space, space_ref => Space;
        new_text, is_text, text, text_ref => PlainText(c: char);
        new_ret, is_ret, ret, ret_ref => Return;
        new_sret, is_sret, sret, sret_ref => ShiftReturn;
        new_esc, is_esc, esc, esc_ref => Escape;
    }}
}

pub(crate) static SYNC_BUF: LazyLock<Mutex<Stdout>> = LazyLock::new(|| Mutex::new(io::stdout()));

pub(crate) async fn render(mut state: State) -> anyhow::Result<()> {
    let mut stdout = SYNC_BUF.lock().await;

    loop {
        // NOTE: there may be a better way of doing this, as I don't believe constant
        // moves on every loop iteration are even remotely optimal. The reason why we
        // have to move everything into the closure is because `tokio` requires
        // everythin that is captured to be captured for `'static`.
        (state, stdout) = task::spawn_blocking(move || draw_screen(state, stdout)).await??;

        if update(&mut state).await.should_terminate() {
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

pub(crate) async fn update(state: &mut State) -> Termination<()> {
    let res = state.receive_event();

    if res.should_terminate() {
        return Termination::terminate();
    }

    // NOTE: this won't panic because the inner representation for a state of
    // termination has already been proved to not be a termination state by the
    // above condition.
    state.update(res.into_inner()).await;

    Termination::keep_going(())
}

pub(crate) async fn handle_input(channel: UnboundedSender<RawUserEvent>) -> anyhow::Result<()> {
    let mut event_stream = EventStream::new().fuse();

    while let Some(event) = event_stream.next().await {
        match event? {
            Event::Key(KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::new_space()),
            Event::Key(KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::new_text(c)),
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::new_ret()),
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::SHIFT,
                ..
            }) => _ = channel.send(RawUserEvent::new_sret()),
            Event::Key(KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..
            }) => _ = channel.send(RawUserEvent::new_esc()),

            // The termination event, which should break out of the loop and drop the producer end
            // of the channel to have the receiver end indicate termination to the task managing it.
            // This tries to replicate getting sent SIGINT or EOF, which are likely the most common
            // ways of triggering relatively smooth termination in the absence of an explicit
            // mechanism to do so. Such control sequences/signals are not available through the
            // keyboard in raw mode.
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
    let (state, events_tx) = State::new(parsed_constants);

    prepare_space().await?;

    let input_handler = task::spawn(handle_input(events_tx));
    let renderer = task::spawn(render(state));

    future::try_join(input_handler, renderer)
        .await
        .map(|(res1, res2)| res1.and(res2))?
}
