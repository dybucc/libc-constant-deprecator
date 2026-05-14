#![feature(try_blocks, async_fn_traits, unboxed_closures)]

use std::{
    borrow::{Borrow, Cow},
    cmp::Ordering,
    debug_assert_matches, env,
    fmt::{self, Debug, Display, Formatter},
    fs::File,
    io::{self as std_io, BufWriter as StdBufWriter, Write},
    ops::{Bound, ControlFlow, Deref, DerefMut, Range, RangeBounds},
    path::PathBuf,
    sync::{Mutex as StdMutex, OnceLock},
    time::Duration,
};

use anyhow::anyhow;
use clap::Parser;
use crossterm::{
    Command,
    cursor::{
        self, Hide, MoveRight, MoveToNextLine, MoveToPreviousLine, RestorePosition, SavePosition,
        SetCursorStyle, Show,
    },
    event::{
        Event, EventStream, KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    style::{ContentStyle, Print, StyledContent, Stylize},
    terminal::{self, Clear, ClearType},
};
use futures::{StreamExt, future};
use libc_constant_deprecator_lib::{BorrowedContainer, Const, ConstContainer, Visit};
use proc_macro2::Ident;
use tester_impl::defer_drm;
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender, error::TryRecvError},
    task, time,
};
use tracing::{info, info_span};

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
            $($wrap_field:tt: $wrap_type:ty),+ $(,)?
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

        // NOTE: the impl for the internal representation is verbosily repeated
        // for both the public type implementing the kind and the internal type
        // denoting the variants because its body requires metavariable
        // expansion at multiple levels of depth.
        #[allow(
            dead_code,
            unused_variables,
            reason = "The macro requires having some type parameters expand to semantically \
                      unignored parameters because I can't think of a way to modify the provided \
                      parameter token trees."
        )]
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

        #[allow(
            dead_code,
            unused_variables,
            reason = "The macro requires having some type parameters expand to semantically \
                      unignored parameters because I can't think of a way to modify the provided \
                      parameter token trees."
        )]
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
macro_rules! repr_impl {
    ($over_t:tt => { // The type used for the internal representation.
        $(
            // The constructor that yields the wrapper type over the internal representation.
            $ctor:ident $({
                // The additional fields that the wrapper type may have been created with.
                $($ctor_field:tt $(: $ctor_field_init:expr)?),* $(,)?
            })?,
            $fallible_is:ident, // The checker operation to ensure a certain variant is within it.
            $infallible_is:ident, // The infallible operation that unwraps a variant or panics.
            $infallible_is_ref:ident, // The non-consuming shared reference getter.
            $infallible_is_mut:ident // The non-consuming exclusive reference getter.
            => $var:tt $(($($tuple:tt: $tuple_t:ty),*))? $({$($field:tt: $field_t:ty),*})?
        );+ ;
    }) => {
        $(
            #[allow(
                dead_code,
                unused_variables,
                reason = "The macro requires having some type parameters expand to semantically \
                          unignored parameters because I can't think of a way to modify the \
                          provided parameter token trees."
            )]
            pub(crate) fn $ctor($($($tuple: $tuple_t),*)? $($($field: $field_t),*)?) -> Self {
                Self {
                    // The field for the internal representation that is always present.
                    repr: $over_t::$var $(($($tuple),*))? $({$($field),*})?,
                    // Other fields that we may have added as part of the wrapper type's
                    // information.
                    $($($ctor_field $(: $ctor_field_init)?),*)?
                }
            }

            // NOTE: of the below two conditionally-expanded token streams, only one of them will
            // expand, and so the actual routine identifiers are the same. This is because the first
            // corresponds with having an enum variant that contains some tuple field(s,) while the
            // latter corresponds with having an enum variant that contains some struct-like fields.

            $(
                #[allow(
                    dead_code,
                    unused_variables,
                    unused_parens,
                    reason = "The macro requires having some type parameters expand to \
                              semantically unignored parameters because I can't think of a way to \
                              modify the provided parameter token trees."
                )]
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

                #[allow(
                    dead_code,
                    unused_variables,
                    unused_parens,
                    reason = "The macro requires having some type parameters expand to \
                              semantically unignored parameters because I can't think of a way to \
                              modify the provided parameter token trees."
                )]
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

                #[allow(
                    dead_code,
                    unused_variables,
                    unused_parens,
                    reason = "The macro requires having some type parameters expand to \
                              semantically unignored parameters because I can't think of a way to \
                              modify the provided parameter token trees."
                )]
                #[track_caller]
                pub(crate) fn $infallible_is_mut(&mut self) -> ($(&mut $tuple_t),*) {
                    if let $over_t::$var(res) = &mut self.repr {
                        res
                    } else {
                        let mut err = String::new();
                        $(err.push_str(stringify!($tuple_t));)*
                        panic!("variant did not contain {}", err);
                    }
                }
            )?

            $(
                #[allow(
                    dead_code,
                    unused_variables,
                    unused_parens,
                    reason = "The macro requires having some type parameters expand to \
                              semantically unignored parameters because I can't think of a way to \
                              modify the provided parameter token trees."
                )]
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

                #[allow(
                    dead_code,
                    unused_variables,
                    unused_parens,
                    reason = "The macro requires having some type parameters expand to \
                              semantically unignored parameters because I can't think of a way to \
                              modify the provided parameter token trees."
                )]
                #[track_caller]
                pub(crate) fn $infallible_is_ref(&self) -> ($(&$field),*) {
                    if let $over_t::$var{$($field:tt),*} = &self.repr {
                        ($($field),*)
                    } else {
                        let mut err = String::new();
                        $(err.push_str(stringify!($field_t));)*
                        panic!("variant did not contain {}", err);
                    }
                }

                #[allow(
                    dead_code,
                    unused_variables,
                    unused_parens,
                    reason = "The macro requires having some type parameters expand to \
                              semantically unignored parameters because I can't think of a way to \
                              modify the provided parameter token trees."
                )]
                #[track_caller]
                pub(crate) fn $infallible_is_mut(&mut self) -> ($(&mut $field),*) {
                    if let $over_t::$var{$($field:tt),*} = &mut self.repr {
                        ($($field),*)
                    } else {
                        let mut err = String::new();
                        $(err.push_str(stringify!($field_t));)*
                        panic!("variant did not contain {}", err);
                    }
                }
            )?

            #[allow(
                dead_code,
                unused_variables,
                reason = "The macro requires having some type parameters expand to semantically \
                          unignored parameters because I can't think of a way to modify the \
                          provided parameter token trees."
            )]
            pub(crate) fn $fallible_is(&self) -> bool {
                matches!(&self.repr, $over_t::$var $(($($tuple),*))? $({$($field),*})?)
            }
        )+
    };
}

// TODO: it may be necessary to add a flag to allow customizing the maximum
// regex compilation limit, even if we allow regexes to compile for up to 2^20
// bytes ~= 1.04 mebibytes.
#[derive(Debug, Parser)]
pub(crate) struct Args {
    /// Path to the `libc` repo. Pass a non-existent directory to clone it to
    /// that directory.
    path: Option<String>,
}

repr! {
    #[derive(Debug, Default, Clone, Copy)]
    SpinnerKind
    #[derive(Debug, Default, Clone, Copy)]
    SpinnerRepr => {
        #[default]
        Vert,
        Left,
        Hor,
        Right,
    }
    #[derive(Debug, Default, Clone, Copy)]
    Spinner
}

impl Display for Spinner {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.repr, f)
    }
}

impl Spinner {
    fn transition(&mut self) -> Self {
        self.repr.transition();

        *self
    }

    // NOTE: I tried getting this to work with a `Cow<'static, str>` but I failed.
    // I'm likely missing some subtleties of the type system, and more specifically,
    // something with subtyping, as the issue is in that just declaring the generic
    // type parameter for `F` to contain a `'static` in the `Cow` is not general
    // enough for the type system. Apparently, it requires that the lifetime
    // associated with `Cow` is valid for all possible lifetimes. I then tried to
    // replace the `'static` in `Cow` with a higher-ranked trait bound that
    // satisfied the type system, but that for some reason, complained at call site
    // that the parmater we are getting into the routine escapes it whenever it is
    // that we call `send` on such parameter (the unbounded transmitter.) For the
    // time being, I have gone with a slightly less efficient implementation, but
    // one that at least type checks.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn run_while<
        T: Send + 'static,
        F: AsyncFnOnce(UnboundedSender<String>) -> anyhow::Result<T>,
    >(
        mut stdout: impl Write + Send + 'static,
        f: F,
    ) -> anyhow::Result<T>
    where
        <F as AsyncFnOnce<(UnboundedSender<String>,)>>::CallOnceFuture: Send + 'static,
    {
        let (tx, mut rx) = mpsc::unbounded_channel();

        let worker = task::spawn(f(tx));
        let spinner = task::spawn(async move {
            info!("started spinner routine");

            let mut spinner = Self::default();
            let mut current_msg: Cow<'static, str> = "".into();

            task::block_in_place(|| {
                crossterm::execute!(
                    stdout,
                    Hide,
                    RestorePosition,
                    Clear(ClearType::FromCursorDown),
                )
            })?;

            loop {
                match rx.try_recv() {
                    Ok(msg) => {
                        info!(new_message = true, message = msg);

                        current_msg = msg.into();
                    }
                    Err(TryRecvError::Empty) => (),
                    Err(TryRecvError::Disconnected) => break,
                }

                task::block_in_place(|| {
                    crossterm::execute!(
                        stdout,
                        Clear(ClearType::CurrentLine),
                        Print(fmt::from_fn(|f| { write!(f, "{spinner} {current_msg}") })),
                        RestorePosition,
                    )
                })?;

                spinner.transition();
                info!("transitioned to spinner: {spinner}");

                time::sleep(Duration::from_millis(128)).await;
            }

            info!("finished spinner task");

            anyhow::Ok(())
        });

        future::try_join(worker, spinner)
            .await
            .map(|(worker_res, spinner_res)| spinner_res.and(worker_res))?
    }
}

impl Display for SpinnerRepr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vert => write!(f, "|"),
            Self::Left => write!(f, "/"),
            Self::Hor => write!(f, "-"),
            Self::Right => write!(f, r"\"),
        }
    }
}

impl SpinnerRepr {
    fn transition(&mut self) {
        *self = match self {
            Self::Vert => Self::Left,
            Self::Left => Self::Hor,
            Self::Hor => Self::Right,
            Self::Right => Self::Vert,
        }
    }
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
    repr_impl! { TerminationRepr => {
        terminate, should_terminate, _d, _d, _d => Termination;
        keep_going, shant_terminate, into_inner, inner, inner_mut => NonTermination(t: T);
    }}
}

repr! {
    #[derive(Debug, Clone, Copy)]
    MsgKind
    #[derive(Debug, Clone, Copy)]
    MsgRepr => {
        EffectingChnages,
        FinishedChanges,
    }
    #[derive(Debug, Clone, Copy)]
    Msg
}

impl Msg {
    fn stringified(self) -> &'static str {
        match self.repr {
            MsgRepr::EffectingChnages => "Effecting changes to disk",
            MsgRepr::FinishedChanges => "Finished effecting changes to disk",
        }
    }

    repr_impl! { MsgRepr => {
        effecting_changes, is_effecting, _d, d, _d => EffectingChnages;
        finished, is_finished, _d, d, _d => FinishedChanges;
    }}
}

// NOTE: we use this across different sites that require calling into values
// with a `'static` lifetime, when we really only have a reference that cannot
// escape the scope in which the task with the lifetime bound has been spawned.
#[repr(transparent)]
struct ThreadedPtr<T> {
    repr: *mut T,
}

impl<T> ThreadedPtr<T> {
    fn new(repr: &mut T) -> Self {
        Self { repr }
    }
}

unsafe impl<T> Send for ThreadedPtr<T> {}

unsafe impl<T> Sync for ThreadedPtr<T> {}

impl<T> Deref for ThreadedPtr<T> {
    type Target = *mut T;

    fn deref(&self) -> &Self::Target {
        let Self { repr } = self;

        repr
    }
}

impl<T> DerefMut for ThreadedPtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let Self { repr } = self;

        repr
    }
}

#[derive(Debug)]
pub(crate) struct State {
    effects: UnboundedSender<Msg>,
    events: UnboundedReceiver<RawUserEvent>,
    mode: Mode,
    constants: ConstContainer,
    filter_buf: BorrowedContainer,
    // NOTE: the following invariant holds for the range of selected symbols:
    // - If in normal mode, the range is empty. This, by extension, also holds when entering insert
    //   mode though that matters not because the only actions available in insert mode are mode
    //   switching, searching and prompt/list focus switching.
    // - If in select mode, the range may be empty or not. The logic here is that it starts off
    //   empty as it comes from normal mode, but can be extended within select mode. A consequence
    //   of this is that range selection is reset back to an empty range once the user exits out of
    //   select mode.
    selected: Selection,
    prompt: String,
    pos: Position,
}

// NOTE: This serves as an extension trait over the functionality already
// offered in `RangeBounds`, only instead of returning an immutable reference
// within the bound, it returns a mutable reference to the underlying `Idx`
// type. This can also be accomplished with a `Bound<T>::as_mut()`, but that
// method appends another level of indirection to the inner `T` if it is a `&T`
// and not an owned `T` (i.e. you end up with a `Bound<&mut &T>` instead of
// `Bound<&mut T>` where `T` is owned in this latter case.)
#[expect(
    dead_code,
    reason = "Some trait methods exist purely for symmetry's sake."
)]
trait RangeBoundsExt<T> {
    fn start_bound_mut(&mut self) -> Bound<&mut T>;
    fn end_bound_mut(&mut self) -> Bound<&mut T>;
}

macro_rules! range_bounds_impl {
    (@body => $bound:expr, $bound_id:expr) => {
        $bound.map(|_| ()).map(|_| $bound_id)
    };
    ($t:tt$(<T $(, $it:tt)*>)?) => {
        impl<T $(, $($it),*)?> RangeBoundsExt<T> for $t$(<T$(, $($it),+)?>)? {
            fn start_bound_mut(&mut self) -> Bound<&mut T> {
                range_bounds_impl! { @body => self.start_bound(), &mut self.start}
            }

            fn end_bound_mut(&mut self) -> Bound<&mut T> {
                range_bounds_impl! { @body => self.end_bound(), &mut self.end }
            }
        }
    };
}

range_bounds_impl! { Range<T> }

#[derive(Debug, Default, Clone)]
pub(crate) struct Selection {
    repr: SelectionRange,
    pivot: u16,
}

#[expect(
    dead_code,
    reason = "It's mostly just accessor and mutator methods that exist for symmetry's sake."
)]
impl Selection {
    pub(crate) fn with_pivot(pivot: u16) -> Self {
        Self {
            pivot,
            repr: SelectionRange::new(pivot, pivot + 1),
        }
    }

    pub(crate) fn map_ref<T>(&self, f: impl FnOnce(&Self) -> T) -> T {
        f(self)
    }

    pub(crate) fn into_range(self) -> Range<u16> {
        let Self {
            repr: SelectionRange { repr },
            ..
        } = self;

        repr
    }

    pub(crate) fn range(&self) -> &Range<u16> {
        let Self {
            repr: SelectionRange { repr },
            ..
        } = self;

        repr
    }

    pub(crate) fn range_mut(&mut self) -> &mut Range<u16> {
        let Self {
            repr: SelectionRange { repr },
            ..
        } = self;

        repr
    }

    #[cfg_attr(debug_assertions, tracing::instrument(skip(self, dir)))]
    pub(crate) fn extend(
        &mut self,
        dir: Dir,
        #[cfg(not(debug_assertions))] pos: impl Borrow<Position>,
        #[cfg(debug_assertions)] pos: impl Borrow<Position> + Debug,
    ) {
        let Self { repr, pivot } = self;
        let pos = pos.borrow();

        info!(pivot);

        // FIXME: everything's patched except for an edge case when sitting at the end
        // of the list, where extension should not happen downwards, but is apparently
        // being allowed.
        match dir.kind() {
            DirKind::Up if pos.into_list() < *pivot => {
                if let start = repr.start_mut()
                    && *start > 0
                {
                    *start -= 1;
                }
            }
            DirKind::Up
                if let (start, end) = (*repr.start(), repr.end_mut())
                    && *end > start + 1 =>
            {
                *end -= 1;
            }

            DirKind::Down
                if pos.into_list() <= *pivot
                    && let start = repr.start_mut()
                    && *start < 9 =>
            {
                *start += 1;
            }
            DirKind::Down
                if let end = repr.end_mut()
                    && *end < 10 =>
            {
                *end += 1;
            }
            _ => (),
        }
    }
}

impl RangeBounds<u16> for Selection {
    fn start_bound(&self) -> Bound<&u16> {
        let Self { repr, .. } = self;

        repr.repr.start_bound()
    }

    fn end_bound(&self) -> Bound<&u16> {
        let Self {
            repr: SelectionRange { repr },
            ..
        } = self;

        repr.end_bound()
    }
}

#[derive(Debug, Clone)]
struct SelectionRange {
    repr: Range<u16>,
}

#[expect(
    dead_code,
    reason = "It's mostly just accessor and mutator methods that exist for symmetry's sake."
)]
impl SelectionRange {
    fn new(start: u16, end: u16) -> Self {
        Self {
            repr: Range { start, end },
        }
    }

    fn start_bound_mut(&mut self) -> Bound<&mut u16> {
        let Self { repr } = self;

        repr.start_bound_mut()
    }

    fn start(&self) -> &u16 {
        let Self { repr } = self;

        &repr.start
    }

    fn start_mut(&mut self) -> &mut u16 {
        let Self { repr } = self;

        &mut repr.start
    }

    fn end_bound_mut(&mut self) -> Bound<&mut u16> {
        let Self { repr } = self;

        repr.start_bound_mut()
    }

    fn end(&self) -> &u16 {
        let Self { repr } = self;

        &repr.end
    }

    fn end_mut(&mut self) -> &mut u16 {
        let Self { repr } = self;

        &mut repr.end
    }

    fn into_inner(self) -> Range<u16> {
        let Self { repr } = self;

        repr
    }

    fn range(&self) -> &Range<u16> {
        let Self { repr } = self;

        repr
    }

    fn range_mut(&mut self) -> &mut Range<u16> {
        let Self { repr } = self;

        repr
    }
}

// NOTE: we need to implement `Default` manually because otherwise it uses the
// `Default` impl of the polymorphic type, which in this case is `u16`. For our
// invariants to be upkept, we require the selection to always be either empty
// or non-empty, but never with the same lower and upper bounds.
impl Default for SelectionRange {
    fn default() -> Self {
        Self {
            repr: Range { start: 0, end: 1 },
        }
    }
}

repr! {
    #[derive(Debug, Clone, Copy)]
    DirKind
    #[derive(Debug, Clone, Copy)]
    DirRepr => {
        Up,
        Down
    }
    #[derive(Debug, Clone, Copy)]
    Dir
}

impl Dir {
    repr_impl! { DirRepr => {
        new_up, is_up, _d, _d, _d => Up;
        new_down, is_down, _d, _d, _d => Down;
    }}
}

// NOTE: we will require modifying some of the routines here once we get to
// implementing scrolling in the 10-row list of constant symbols.
impl State {
    #[tracing::instrument(skip_all)]
    fn new(
        constants: ConstContainer,
    ) -> (Self, UnboundedSender<RawUserEvent>, UnboundedReceiver<Msg>) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (msg_tx, msg_rx) = mpsc::unbounded_channel();

        (
            Self {
                effects: msg_tx,
                events: events_rx,
                mode: Mode::default(),
                filter_buf: constants.borrowed(),
                constants,
                selected: Selection::default(),
                prompt: String::default(),
                pos: Position::default(),
            },
            events_tx,
            msg_rx,
        )
    }

    // TODO: refactor this routine into smaller chunks. Recall that we already
    // attempted getting the function calls for mode switching separated into a
    // table of closures/function pointers and that did not work out too well.
    #[tracing::instrument(skip_all, err(level = "info"))]
    pub(crate) fn update(&mut self, event: Option<UserEvent>) -> anyhow::Result<()> {
        let Self {
            filter_buf,
            selected,
            prompt,
            constants,
            mode,
            pos,
            effects,
            ..
        } = self;

        let Some(event) = event else {
            return Ok(());
        };

        match event.kind() {
            UserEventKind::TextualInput => {
                let current_col = usize::from(pos.into_prompt());

                if current_col == prompt.len() {
                    prompt.push(event.into_text());
                } else {
                    prompt.insert(current_col, event.into_text());
                }

                pos.advance(filter_buf.len());
            }
            UserEventKind::Pop => {
                let current_col = usize::from(pos.into_prompt());

                if current_col == prompt.len() {
                    prompt.pop();
                } else {
                    prompt.remove(current_col);
                }

                pos.retract();
            }
            UserEventKind::Search => {
                macro_rules! info_first_ten {
                    ($intro:expr) => {{
                        if cfg!(debug_assertions) {
                            let span = info_span!($intro);

                            filter_buf.select(
                                0..if let len = filter_buf.len() && len < 10 { len } else { 10 }
                            ).visit(|constant| {
                                info!(parent: &span, "{}", constant.ident());

                                ControlFlow::<(), _>::Continue(())
                            });
                        }
                    }};
                }

                info!(name: "filtering", filtering_event = true);

                info_first_ten!("filter_buf_pre_searching");
                constants.filter_with(&prompt, filter_buf)?;
                info_first_ten!("filter_buf_post_searching");

                *selected = Selection::default();

                if pos.is_list() {
                    *pos = Position::new_list(0);
                }
            }

            // NOTE: toggles all constants because we are either (1) outside select mode, or (2)
            // within it but without a selection ranging beyond a single row.
            UserEventKind::Toggle if mode.is_normal() && all_deprecated(filter_buf) => {
                info!(toggle_mode = "normal", toggle_type = "undeprecation");

                filter_buf.undeprecate();
            }
            UserEventKind::Toggle if mode.is_normal() => {
                info!(toggle_mode = "normal", toggle_type = "deprecation");

                filter_buf.deprecate();
            }
            // NOTE: this uses `BorrowedSubset` to add another degree of refinement to the set of
            // constants currently under consideration. This only happens in select mode, where the
            // range in `selected` may be non-empty.
            UserEventKind::Toggle => {
                info!(toggle_mode = "select");

                toggle_in_select(filter_buf, selected);
            }

            // NOTE: the current solution forces everything to type check by embedding my own
            // reasoning about the way we handle the references that the task captures. Because we
            // know of the drawing function that awaits for this task to finish, we can soundly
            // state that the captured data (raw pointers to be used as references) does not require
            // the lifetime of all captured variables to be `'static`. It is guaranteed (though not
            // a the type level, that the `print_changes` routine will not return until the task
            // spawned here is complete, lest there's some fallible operation that returns early; In
            // that case, it would all depend on the order in which values are dropped and tasks are
            // cancelled.)
            //
            // A less risky solution would instead be to hold on to a shared pointer for the
            // resources we require holding onto this task; Namely, the constants and the channel
            // transmitter, such that we may clone them and thus not allow their ownership and drop
            // glue to be tied to the running state.
            //
            // Ideally, the task would also be stored in a local set that could yield the results of
            // the computations within it in case some write to disk fails (which at present is
            // silently ignored.)
            UserEventKind::Effect => {
                let constants = ThreadedPtr::new(constants);
                let effects = ThreadedPtr::new(effects);

                task::spawn(async move {
                    let (constants, effects) =
                        unsafe { (constants.as_ref_unchecked(), effects.as_ref_unchecked()) };

                    info!("entered write-to-disk task");

                    effects.send(Msg::effecting_changes())?;
                    constants.effect_changes().await?;
                    effects.send(Msg::finished())?;

                    anyhow::Ok(())
                });
            }
            UserEventKind::Clear if !prompt.is_empty() => {
                prompt.clear();

                if pos.is_prompt() {
                    *pos = Position::new_prompt(0);
                }

                constants.filter_with(".*", filter_buf)?;
            }
            UserEventKind::Switch if !filter_buf.is_empty() => match mode.kind() {
                ModeKind::Insert => {
                    *mode = Mode::new_normal();

                    pos.transition();
                }
                ModeKind::Normal => pos.transition(),
                ModeKind::Select => {
                    *mode = Mode::new_normal();
                    *selected = Selection::default();

                    pos.transition();
                }
            },
            UserEventKind::ModeAction => {
                let action = event.into_action();

                match action.kind() {
                    ModeActionKind::ModeSwitch => {
                        let new_mode = action.into_mode();

                        match (mode.kind(), new_mode.kind()) {
                            // Insert to normal mode.
                            (ModeKind::Insert, ModeKind::Normal) => *mode = new_mode,

                            // Normal mode to other modes.
                            (ModeKind::Normal, ModeKind::Insert) => {
                                *mode = new_mode;

                                // NOTE: because normal mode can be used in both the prompt and the
                                // list, but insert mode can only be used in the prompt, we require
                                // switching over the current cursor position to the prompt if we're
                                // currently in the list of filtered symbols.
                                if pos.is_list() {
                                    pos.transition();
                                }
                            }
                            (ModeKind::Normal, ModeKind::Select) if pos.is_list() => {
                                *mode = new_mode;

                                // NOTE: because selecting requires keeping track of which element
                                // the selection started at to determine which of the bounds should
                                // be changes at a given time, we have to set the current pivot to
                                // be the current cursor position. The associated function
                                // `with_pivot` will return a `Selection` that starts its range in
                                // the current element, and has set the pivot the current element.
                                *selected = Selection::with_pivot(pos.into_list());
                            }

                            // Select mode to other modes.
                            (ModeKind::Select, ModeKind::Insert) => {
                                *mode = new_mode;
                                *selected = Selection::default();

                                // NOTE: Select mode can only be used in the list of filtered
                                // symbols, and itself can only be accessed when the cursor is
                                // currently navigating the list, so going into insert mode, which
                                // can only be used in the prompt, always requires transitioning the
                                // cursor position.
                                pos.transition();
                            }
                            (ModeKind::Select, ModeKind::Normal) => {
                                *mode = new_mode;
                                *selected = Selection::default();
                            }

                            // NOTE: other cases include non-sensical transitions between the same
                            // one mode. Other cases to note include:
                            // + Going from normal mode to select mode while navigating the prompt
                            //   in normal mode. This is not possible, because going into select
                            //   mode requires focus to be in the list container.
                            // + Going from insert to select mode, which is not possible because
                            //   select mode is only reachable through normal mode, and more
                            //   specifically, while focus is on the list container.
                            _ => (),
                        }
                    }

                    // NOTE: which one of the row or column should be modified is already handled in
                    // the corresponding impl of `Add` and `Sub` for `Position`. This allows us to
                    // converge events into two basic operations over an x-axis and a y-axis.
                    ModeActionKind::GoLeft if mode.is_normal() && pos.is_prompt() => {
                        debug_assert_matches!(
                            usize::from(pos.into_prompt()).cmp(&prompt.len()),
                            Ordering::Less | Ordering::Equal,
                            "prompt position in terminal grid should always be less than or equal \
                             to the prompt string length"
                        );

                        pos.retract();
                    }
                    ModeActionKind::GoRight if mode.is_normal() && pos.is_prompt() => {
                        debug_assert_matches!(
                            usize::from(pos.into_prompt()).cmp(&prompt.len()),
                            Ordering::Less | Ordering::Equal,
                            "prompt position in terminal grid should always be less than or equal \
                             to the prompt string length"
                        );

                        // NOTE: this ensures moving right does moves only one character past the
                        // last character written in the prompt, such that entering insert mode can
                        // remove that one character.
                        if usize::from(pos.into_prompt()) == prompt.len() {
                            return Ok(());
                        }

                        pos.advance(filter_buf.len());
                    }

                    ModeActionKind::GoUp if mode.is_normal() && pos.is_list() => pos.retract(),
                    ModeActionKind::GoDown if mode.is_normal() && pos.is_list() => {
                        pos.advance(filter_buf.len());
                    }

                    ModeActionKind::GoUp if mode.is_select() => {
                        pos.retract();

                        selected.extend(Dir::new_up(), pos);
                    }
                    ModeActionKind::GoDown if mode.is_select() => {
                        pos.advance(filter_buf.len());

                        selected.extend(Dir::new_down(), pos);
                    }

                    // NOTE: ignored cases include motion bindings while in insert mode, which we do
                    // not process as navigation keys are always interpreted as textual input.
                    _ => (),
                }
            }

            // NOTE: ignored cases include:
            // + Issuing a clear command without there being anything in the prompt, which also
            //   triggers a wildcard regex. We prefer not to call into that regex if not necessary
            //   (even if it gets cached past the first clear command.)
            // + Attempting to swap between modes when there are no results in the list of filtered
            //   symbols.
            _ => (),
        }

        Ok(())
    }

    pub(crate) async fn receive_event(&mut self) -> Termination<Option<UserEvent>> {
        let Self { events, mode, .. } = self;

        // NOTE: the point here is to sleep until a new event is made available, such
        // that the `tokio` executor can just keep everything on hold, or otherwise
        // yield to the input handler, which is the only other task that it can switch
        // to once this `.await` point reaches the topmost "task gathering point."
        match events.recv().await {
            Some(event) => Termination::keep_going(mode.interpret(event)),
            None => Termination::terminate(),
        }
    }

    // TODO: implement support for showing the current path to the constant besides
    // the constant itself.
    #[tracing::instrument(skip_all, err(level = "info"))]
    async fn draw(
        &mut self,
        mut stdout: impl Write + Send + 'static,
        effecting_changes: &mut UnboundedReceiver<Msg>,
    ) -> anyhow::Result<()> {
        let Self {
            mode,
            filter_buf,
            selected,
            prompt,
            pos,
            ..
        } = self;

        // NOTE: we proceed by first drawing all static contents to the terminal buffer,
        // to then highlight and/or reposition the cursor according to the current
        // running state.
        //
        // For that, we consider two types of routines; The printers and the finalizers.
        // The printers perform operations that have a side effect but on exit will
        // guarantee that the terminal cursor is at the very first column of the line
        // where the prompt is. These are also the routines in charge of getting
        // everything initially layed out on-screen.
        //
        // The finalizers do something similar, but are meant to be run last to provide
        // the last tidbits of visual feedback required for each one of the modes. This
        // includes highlighting items in the list, as well as moving the cursor
        // wherever it is that the `pos` field in the running state indicates.
        //
        // Prior to all of that, we check if the running state is currently effecting
        // changes to disk, as that requires completely wiping the screen and printing a
        // progress report with the `Spinner` type.

        if let Ok(init_msg) = effecting_changes.try_recv() {
            info!("started to effect changes: {:?}", init_msg.stringified());

            let stdout = ThreadedPtr::new(&mut stdout);
            let effecting_changes = ThreadedPtr::new(effecting_changes);

            print_changes(
                unsafe { stdout.as_mut_unchecked() },
                init_msg,
                effecting_changes,
            )
            .await?;
        }

        print_reset(&mut stdout)?;
        print_prompt(&mut stdout, prompt)?;

        if filter_buf.is_empty() {
            print_empty(&mut stdout)?;

            // NOTE: this ensures that if we are in the list of constants, the finalizer
            // routine that comes up next does not bother rendering the list of constants,
            // for there's none.
            if pos.is_list() {
                pos.transition();
            }
        } else {
            print_list(&mut stdout, &select_first_ten(filter_buf))?;
        }

        match (mode.kind(), pos.kind()) {
            (ModeKind::Insert, PositionKind::Prompt) => {
                finalize_insert_prompt(&mut stdout, pos.into_prompt())?;
            }
            (ModeKind::Normal, PositionKind::Prompt) => {
                finalize_normal_prompt(&mut stdout, pos.into_prompt())?;
            }
            (ModeKind::Normal, PositionKind::List) => {
                finalize_normal_list(&mut stdout, pos.into_list(), &select_first_ten(filter_buf))?;
            }
            (ModeKind::Select, PositionKind::List) => {
                finalize_select_list(
                    &mut stdout,
                    pos.into_list(),
                    &select_first_ten(filter_buf),
                    selected,
                )?;
            }

            // NOTE: ignored cases include being in insert mode while in the list, and being
            // in select mode while in the prompt, both of which are
            // logically impossible.
            _ => (),
        }

        stdout.flush().map_err(Into::into)
    }
}

fn select_first_ten(buf: &mut BorrowedContainer) -> impl Visit {
    buf.select(
        0..if let len = buf.len()
            && len < 10
        {
            len
        } else {
            10
        },
    )
}

#[tracing::instrument(skip(stdout, effecting_changes), err(level = "info"))]
async fn print_changes(
    stdout: impl Write + Send + 'static,
    init_msg: Msg,
    effecting_changes: ThreadedPtr<UnboundedReceiver<Msg>>,
) -> anyhow::Result<()> {
    Spinner::run_while(stdout, async move |tx| {
        let effecting_changes = unsafe { effecting_changes.as_mut_unchecked() };

        tx.send(init_msg.stringified().into())?;

        while let Some(msg) = effecting_changes.recv().await {
            if let MsgKind::EffectingChnages = msg.kind() {
                tx.send(msg.stringified().into())?;
            } else {
                info!("done effecting changes");
                tx.send(msg.stringified().into())?;

                break;
            }
        }

        anyhow::Ok(())
    })
    .await
}

fn print_empty(mut stdout: impl Write) -> anyhow::Result<()> {
    crossterm::queue!(
        stdout,
        MoveToNextLine(1),
        Print("(no constants matched the regex)"),
        RestorePosition,
    )
    .map_err(Into::into)
}

#[tracing::instrument(skip_all, err(level = "info"))]
fn print_reset(mut stdout: impl Write) -> anyhow::Result<()> {
    crossterm::queue!(
        stdout,
        Hide,
        RestorePosition,
        Clear(ClearType::FromCursorDown)
    )
    .map_err(Into::into)
}

#[tracing::instrument(skip_all, err(level = "info"))]
fn print_prompt(mut stdout: impl Write, prompt: impl AsRef<str>) -> anyhow::Result<()> {
    crossterm::queue!(
        stdout,
        Print(fmt::from_fn(|f| write!(f, "> {}", prompt.as_ref()))),
        RestorePosition,
    )
    .map_err(Into::into)
}

// TODO: this will need a refactor once scrolling is implemented.
#[tracing::instrument(skip_all, err(level = "info"))]
fn print_list(mut stdout: impl Write, list: &impl Visit) -> anyhow::Result<()> {
    crossterm::queue!(stdout, MoveToNextLine(1))?;

    list.visit(|constant| {
        if let Err(err) = try {
            crossterm::queue!(
                stdout,
                StylizedSymbol::from_constant(constant).into_command(),
            )?;

            crossterm::queue!(stdout, MoveToNextLine(1))?;
        } {
            ControlFlow::Break(err.into())
        } else {
            ControlFlow::Continue(())
        }
    })
    .flatten()
    .map(Into::into)
    .map_or_else(|| anyhow::Ok(()), Err)?;

    crossterm::queue!(stdout, RestorePosition).map_err(Into::into)
}

#[tracing::instrument(skip(stdout), err(level = "info"))]
fn finalize_insert_prompt(mut stdout: impl Write, off: u16) -> anyhow::Result<()> {
    crossterm::queue!(stdout, MoveRight(2 + off), SetCursorStyle::SteadyBar, Show)
        .map_err(Into::into)
}

#[tracing::instrument(skip(stdout), err(level = "info"))]
fn finalize_normal_prompt(mut stdout: impl Write, off: u16) -> anyhow::Result<()> {
    crossterm::queue!(
        stdout,
        MoveRight(2 + off),
        SetCursorStyle::SteadyBlock,
        Show,
    )
    .map_err(Into::into)
}

// TODO: this is going to need a refactor once scrolling is implemented but this
// particular routine should likely benefit from the refactor in the
// `print_list` routine for the same scrollign purposes.
#[tracing::instrument(skip(stdout, list), err(level = "info"))]
fn finalize_normal_list(mut stdout: impl Write, off: u16, list: &impl Visit) -> anyhow::Result<()> {
    // NOTE: we have to keep this declaration outside the macro invocation because
    // the result that we propagate has as an `Err` variant a `anyhow::Error`, which
    // is not convertible to the `std::io::Result` that the closure handled within
    // the macro invocation uses (the command is issued within a call to `and_then`
    // after having used the writer we pass in first (here `stdout`).)
    let cmd = list
        .find_indexed(off, |constant| {
            StylizedSymbol::from_constant(constant)
                .for_ident(ContentStyle::new().bold().underlined())
                .for_mark(ContentStyle::new().bold().underlined())
        })
        .ok_or_else(|| {
            anyhow!("offset into visual list of symbols was not within bounds of inner list")
        })?
        .into_command();

    crossterm::queue!(stdout, MoveToNextLine(1 + off), cmd).map_err(Into::into)
}

// TODO: this is going to need a refactor once scrolling is implemented but this
// particular routine should likely benefit from the refactor in the
// `print_list` routine for the same scrollign purposes.
#[tracing::instrument(skip(stdout, list), err(level = "info"))]
fn finalize_select_list(
    mut stdout: impl Write,
    off: u16,
    list: &impl Visit,
    selected: &Selection,
) -> anyhow::Result<()> {
    let Range { start, end } = *selected.range();

    info!(selection_start = start, selection_end = end);

    // NOTE: we move past however as many constants are not part of the selection,
    // plus the "base position" required to reach the list of symbols from the
    // prompt (1 row.)
    crossterm::queue!(stdout, MoveToNextLine(1 + start))?;

    list.select(selected.range(), |constant| {
        crossterm::queue!(
            stdout,
            StylizedSymbol::from_constant(constant)
                .for_mark(ContentStyle::new().bold().underlined())
                .dim()
                .into_command(),
            MoveToNextLine(1),
        )
    })
    .map_or_else(|| anyhow::Ok(()), |res| res.map(|_| ()).map_err(Into::into))?;

    list.find_indexed(off, |constant| {
        crossterm::queue!(
            stdout,
            RestorePosition,
            MoveToNextLine(1 + off),
            StylizedSymbol::from_constant(constant)
                .for_mark(ContentStyle::new().bold().underlined())
                .dim()
                .bold()
                .underlined()
                .into_command(),
        )
    })
    .map_or_else(|| anyhow::Ok(()), |res| res.map_err(Into::into))?;

    crossterm::queue!(stdout, RestorePosition, MoveToNextLine(1 + off),).map_err(Into::into)
}

#[derive(Debug, Clone, Copy)]
struct StylizedSymbol<T: Display> {
    repr: T,
    deprecated: bool,
    ident_style: ContentStyle,
    mark_style: ContentStyle,
}

#[expect(dead_code, reason = "Some methods exist for symmetry's sake.")]
impl StylizedSymbol<Ident> {
    fn from_constant(sym: &Const) -> Self {
        Self {
            repr: sym.ident().clone(),
            deprecated: sym.is_deprecated(),
            ident_style: ContentStyle::new(),
            mark_style: ContentStyle::new(),
        }
    }

    fn ident(&mut self) -> StylizedIdent<'_> {
        let Self { ident_style, .. } = self;

        StylizedIdent::new(ident_style)
    }

    fn mark(&mut self) -> StylizedMark<'_> {
        let Self { ident_style, .. } = self;

        StylizedMark::new(ident_style)
    }

    fn for_mark(mut self, style: ContentStyle) -> Self {
        let Self { mark_style, .. } = &mut self;

        *mark_style = style;

        self
    }

    fn for_ident(mut self, style: ContentStyle) -> Self {
        let Self { ident_style, .. } = &mut self;

        *ident_style = style;

        self
    }
}

impl<T: Display> StylizedSymbol<T> {
    // NOTE: we could implement `crossterm::Command` directly for `Self` and build
    // up the same `PrintStyledContent` there, but the required method for that
    // trait takes a shared reference to `Self`, which means we are forced to both
    // have a mandatory clone and a trait bound on the polymorphic type `T` for
    // `Clone`.
    fn into_command(self) -> impl Command {
        let Self {
            repr,
            deprecated,
            ident_style,
            mark_style,
        } = self;

        let ident = StyledContent::new(ident_style, repr);

        Print(fmt::from_fn(move |f| {
            write!(
                f,
                "{} {ident}",
                StyledContent::new(mark_style, if deprecated { "[X]" } else { "[ ]" }),
            )
        }))
    }
}

#[derive(Debug)]
struct StylizedIdent<'a> {
    style: &'a mut ContentStyle,
}

#[derive(Debug)]
struct StylizedMark<'a> {
    style: &'a mut ContentStyle,
}

macro_rules! style_impl {
    (@fn @spec) => {
        fn new(style: &'a mut ContentStyle) -> Self {
            Self { style }
        }
    };
    (@impl @ $spec:tt) => {
        impl<'a> $spec<'a> {
            style_impl! { @fn @spec }
        }
    };
    (@body @spec => $self:expr) => {
        let Self { style } = $self;

        style
    };
    (@trait @ $spec:tt) => {
        impl<'a> AsRef<ContentStyle> for $spec<'a> {
            fn as_ref(&self) -> &ContentStyle {
                style_impl! { @body @spec => self }
            }
        }

        impl<'a> AsMut<ContentStyle> for $spec<'a> {
            fn as_mut(&mut self) -> &mut ContentStyle {
                style_impl! { @body @spec => self }
            }
        }

        impl<'a> Stylize for $spec<'a> {
            type Styled = Self;

            fn stylize(self) -> Self::Styled {
                self
            }
        }
    };
    (@body => $self:expr) => {
        let Self { ident_style, .. } = $self;

        ident_style
    };
    () => {
        impl<T: Display> AsRef<ContentStyle> for StylizedSymbol<T> {
            fn as_ref(&self) -> &ContentStyle {
                style_impl! { @body => self }
            }
        }

        impl<T: Display> AsMut<ContentStyle> for StylizedSymbol<T> {
            fn as_mut(&mut self) -> &mut ContentStyle {
                style_impl! { @body => self }
            }
        }

        impl<T: Display> Stylize for StylizedSymbol<T> {
            type Styled = Self;

            fn stylize(self) -> Self::Styled {
                self
            }
        }

        style_impl! { @impl @StylizedIdent }
        style_impl! { @impl @StylizedMark }

        style_impl! { @trait @StylizedIdent }
        style_impl! { @trait @StylizedMark }
    };
}

style_impl! {}

#[tracing::instrument(skip_all)]
fn toggle_in_select(filter_buf: &mut BorrowedContainer, selected: &Selection) {
    let mut selected = filter_buf.select(selected.range());

    if all_deprecated(&selected) {
        info!(toggle_type = "undeprecation");

        selected.undeprecate();
    } else {
        info!(toggle_type = "deprecation");

        selected.deprecate();
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

// NOTE: the logic here is that beyond the raw row and columns in the terminal
// grid, the position is, at a higher level, either inside the prompt or in the
// list of filtered symbols. If on the former, there is only the column position
// to worry about; If on the latter, there is only the row position to worry
// about. We thus abstract over an absolute position into an offset-based
// postion scheme.
repr! {
    #[derive(Debug, Clone, Copy)]
    PositionKind
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    PositionRepr => {
        Prompt(u16),
        List(u16)
    }
    // NOTE: the raw position in the terminal grid is stored inline as fields of
    // the public type, while the actual, bounded position within the prompt is
    // in the range 0-{terminal column width}; The list of filtered symbols is
    // within range 0-9. These last two correspond with each of the two numbers
    // in the tuple fields of the above two variants of the internal
    // representation type. The absolute position in the terminal grid is given
    // by the below two fields in the wrapper type -- `Position`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    Position {
        row: u16,
        col: u16,
    }
}

impl Position {
    #[expect(
        clippy::match_wildcard_for_single_variants,
        reason = "The note besdes the wildcard case would quite possibly not be read if it \
                  weren't a wildcard."
    )]
    pub(crate) fn retract(&mut self) {
        let Self { repr, row, col } = self;

        match repr {
            PositionRepr::Prompt(prompt_col) => {
                *prompt_col = prompt_col.saturating_sub(1);
                *col = col.saturating_sub(1);
            }
            PositionRepr::List(list_row) if let Some(res) = list_row.checked_sub(1) => {
                *list_row = res;
                *row -= 1;
            }

            // NOTE: ignored cases include contexts where the user is found to be at the start of
            // the list or at the very left of the prompt.
            _ => (),
        }
    }

    pub(crate) fn advance(&mut self, element_count: usize) {
        let Self { repr, row, col } = self;

        // NOTE: this adjusts the maximum allowed number of elements in the list such
        // that if there are fewer than 10 elements, we properly handle that situation.
        let limit = if element_count < 10 {
            element_count
        } else {
            10
        };

        // NOTE: we map the terminal dimensions from 0-indexed space to 1-indexed space,
        // such that we keep magntidues in the operations following consitent (the
        // internal positions and offsets are 0-indexed.)
        let (max_col, max_row) = terminal::size()
            .map(|(max_col, max_row)| (max_col - 1, max_row - 1))
            .unwrap();

        match repr {
            PositionRepr::Prompt(prompt_col)
                if let res = *prompt_col + 1
                    && res < max_col =>
            {
                *prompt_col = res;
                *col += 1;
            }
            PositionRepr::List(list_row)
                if let res = *list_row + 1
                    && usize::from(res) < limit
                    && res < max_row =>
            {
                *list_row = res;
                *row += 1;
            }

            // NOTE: ignored cases include those were the
            _ => (),
        }
    }
}

impl Position {
    pub(crate) fn transition(&mut self) {
        let Self { repr, .. } = *self;

        match repr {
            PositionRepr::Prompt(_) => *self = Self::new_list(0),
            PositionRepr::List(_) => *self = Self::new_prompt(0),
        }
    }

    // NOTE: when constructing a new instance of a position within any one of the
    // layout containers we use as a list, or the prompt, we always require the raw
    // row to be fetched externally because out of the 2-dimensional pieces of state
    // we keep, the row index is the one most not in sync with the higher-level
    // abstraction that is kept in the `repr` field. We speak of "most not in sync"
    // because the raw column index is neither in sync with the high-level
    // abstraction; The former, when focus is on the regex prompt, is always offset
    // by 2 to account for the visual identifiers we add to the prompt. When focus
    // is on the list of filtered symbols, it is offset by 4 to account for the
    // deprecation checkbox we add at the start of each list item.
    //
    // When creatingn a new view into the list of filtered constants with a given
    // starting offset, the raw position in the terminal grid must be set to the
    // start of the prompt, offsetted by a unit to properly reach the "base"
    // coordinate of the list container, and finally offsetted by the provided row
    // offset `row`.
    repr_impl! { PositionRepr => {
        new_prompt {
            row: PROMPT_COORD.get().unwrap().1,
            col: col + 2,
        },
        is_prompt, into_prompt, prompt, prompt_mut => Prompt(col: u16);
        new_list {
            row: PROMPT_COORD.get().unwrap().1 + 1 + row,
            col: u16::default(),
        },
        is_list, into_list, list, list_mut => List(row: u16);
    }}
}

impl Default for Position {
    fn default() -> Self {
        Self::new_prompt(0)
    }
}

// NOTE: this does not seem to be implementable through an automatically derived
// `Default` on `PositionRepr` so we must implement it manually.
impl Default for PositionRepr {
    fn default() -> Self {
        Self::Prompt(0)
    }
}

repr! {
    #[derive(Debug, Default, Clone, Copy)]
    ModeKind
    #[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
    ModeRepr => {
        Insert,
        #[default]
        Normal,
        Select,
    }
    #[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
    Mode
}

impl Mode {
    #[tracing::instrument(skip(self), fields(mode = ?self), ret)]
    pub(crate) fn interpret(self, event: RawUserEvent) -> Option<UserEvent> {
        match (self.kind(), event.kind()) {
            // Insert mode
            (ModeKind::Insert, RawUserEventKind::PlainText) => {
                UserEvent::new_text(event.into_text())
            }
            (ModeKind::Insert, RawUserEventKind::Space) => UserEvent::new_text(' '),
            (ModeKind::Insert, RawUserEventKind::Backspace) => UserEvent::new_pop(),

            // Normal mode.
            (ModeKind::Normal, RawUserEventKind::PlainText)
                if let Some(action) = ModeAction::is_navigation(event.into_text()) =>
            {
                UserEvent::new_action(action)
            }
            (ModeKind::Normal, RawUserEventKind::PlainText)
                if let Some(action) = ModeAction::is_mode_transition(event.into_text()) =>
            {
                UserEvent::new_action(action)
            }
            (ModeKind::Normal, RawUserEventKind::ShiftReturn) => UserEvent::new_effect(),
            (ModeKind::Normal, RawUserEventKind::Escape) => UserEvent::new_clear(),

            // Select mode.
            (ModeKind::Select, RawUserEventKind::PlainText)
                if let Some(action) = ModeAction::is_navigation(event.into_text()) =>
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

            // The one and only event that is allowed in all modes is to switch between the
            // prompt and the list of constants, whichever one it is that the
            // user is in.
            (ModeKind::Insert | ModeKind::Normal | ModeKind::Select, RawUserEventKind::Tab) => {
                UserEvent::new_switch()
            }

            // NOTE: this includes a bunch of cases where input processing shouldn't even be
            // performed, as the combination of mode/event does not make for a logical event to
            // process.
            _ => return None,
        }
        .into()
    }

    repr_impl! { ModeRepr => {
        new_normal, is_normal, _d, _d, _d => Normal;
        new_insert, is_insert, _d, _d, _d => Insert;
        new_select, is_select, _d, _d, _d => Select;
    }}
}

repr! {
    #[derive(Debug, Clone, Copy)]
    UserEventKind
    #[derive(Debug, Clone, Copy)]
    UserEventRepr => {
        // Corresponds with plain text user input at the prompt.
        TextualInput(char),
        // Corresponds with having pressed the backspace key while in insert
        // mode to remove the character currently under the cursor.
        Pop,
        // Is triggered with the return key and should start a filtering event
        // with the current contents of the prompt.
        Search,
        // Is triggered with the space key and should toggle all selected
        // constants' state to "deprecated", unless all selected constants are
        // already deprecated, in which case it should undeprecate them.
        Toggle,
        // Is triggered with the shift + return combo and should effect the
        // changes to disk.
        Effect,
        // Is triggered with the escape key and should clear the currently input
        // regex.
        Clear,
        // Is triggered when the user switches between modes with the TAB key.
        Switch,
        // Is triggered when going from insert mode to normal mode.
        ModeAction(ModeAction),
    }
    #[derive(Debug, Clone, Copy)]
    UserEvent
}

impl UserEvent {
    repr_impl! { UserEventRepr => {
        new_text, is_text, into_text, text, text_mut => TextualInput(c: char);
        new_pop, is_pop, _d, _d, _d => Pop;
        new_action, is_action, into_action, action, action_mut => ModeAction(action: ModeAction);
        new_search, is_search, _d, _d, _d => Search;
        new_toggle, is_toggle, _d, _d, _d => Toggle;
        new_effect, is_effect, _d, _d, _d => Effect;
        new_clear, is_clear, _d, _d, _d => Clear;
        new_switch, is_switch, _d, _d, _d => Switch;
    }}
}

// NOTE: this type serves as a general LUT for all commands involving anything
// but (1) major actions, which are stored inline under `UserEvent`, and (2)
// plain text user input.
repr! {
    #[derive(Debug, Clone, Copy)]
    ModeActionKind
    #[derive(Debug, Clone, Copy)]
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
    #[derive(Debug, Clone, Copy)]
    ModeAction
}

impl ModeAction {
    fn new(repr: ModeActionRepr) -> Self {
        Self { repr }
    }

    repr_impl! { ModeActionRepr => {
        switch_modes, is_mode, into_mode, mode, mode_mut => ModeSwitch(mode: Mode);
        new_left, is_left, _d, _d, _d => GoLeft;
        new_right, is_right, _d, _d, _d => GoRight;
        new_up, is_up, _d, _d, _d => GoUp;
        new_down, is_down, _d, _d, _d => GoDown;
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
        Tab,
        Backspace,
    }
    #[derive(Debug, Clone, Copy)]
    RawUserEvent
}

impl RawUserEvent {
    repr_impl! { RawUserEventRepr => {
        new_space, is_space, _d, _d, _d => Space;
        new_text, is_text, into_text, text, text_mut => PlainText(c: char);
        new_ret, is_ret, _d, _d, _d => Return;
        new_sret, is_sret, _d, _d, _d => ShiftReturn;
        new_esc, is_esc, _d, _d, _d => Escape;
        new_tab, is_tab, _d, _d, _d => Tab;
        new_backspace, is_backspace, _d, _d, _d => Backspace;
    }}
}

#[tracing::instrument(skip_all, err(level = "info"))]
async fn render(
    mut state: State,
    mut effecting_changes: UnboundedReceiver<Msg>,
) -> anyhow::Result<()> {
    loop {
        draw_screen(&mut state, &mut effecting_changes).await?;

        if update(&mut state).await?.should_terminate() {
            break Ok(());
        }
    }
}

#[tracing::instrument(skip_all, err(level = "info"))]
async fn draw_screen(
    state: &mut State,
    effecting_changes: &mut UnboundedReceiver<Msg>,
) -> anyhow::Result<()> {
    info!(redraw = true);

    let stdout = StdBufWriter::new(std_io::stdout());

    state.draw(stdout, effecting_changes).await
}

#[tracing::instrument(skip_all, ret, err(level = "info"))]
async fn update(state: &mut State) -> anyhow::Result<Termination<()>> {
    let res = state.receive_event().await;

    if res.should_terminate() {
        return Ok(Termination::terminate());
    }

    state.update(res.into_inner())?;

    Ok(Termination::keep_going(()))
}

#[tracing::instrument(skip_all, err(level = "info"))]
async fn handle_input(channel: UnboundedSender<RawUserEvent>) -> anyhow::Result<()> {
    let mut event_stream = EventStream::new().fuse();

    while let Some(event) = event_stream.next().await {
        let event = event?;

        info!(event = ?event);

        if event.is_key_press() {
            match event {
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
                    modifiers: KeyModifiers::SHIFT,
                    ..
                }) => _ = channel.send(RawUserEvent::new_sret()),
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    ..
                }) => _ = channel.send(RawUserEvent::new_ret()),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc,
                    modifiers: KeyModifiers::NONE,
                    ..
                }) => _ = channel.send(RawUserEvent::new_esc()),
                Event::Key(KeyEvent {
                    code: KeyCode::Tab,
                    modifiers: KeyModifiers::NONE,
                    ..
                }) => _ = channel.send(RawUserEvent::new_tab()),
                Event::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    modifiers: KeyModifiers::NONE,
                    ..
                }) => _ = channel.send(RawUserEvent::new_backspace()),

                // The termination event, which should break out of the loop and drop the producer
                // end of the channel to have the receiver end indicate termination
                // to the task managing it. NOTE: this tries to replicate getting
                // sent SIGINT or EOF, which are likely the most common ways of
                // triggering relatively smooth termination in the absence of an explicit
                // mechanism to do so in the program. Such control sequences/signals are not
                // available when the terminal emulator is operating in raw mode.
                Event::Key(KeyEvent {
                    code: KeyCode::Char('c' | 'd'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                }) => break,

                _ => (),
            }
        }
    }

    drop(channel);

    Ok(())
}

// NOTE: we use this while transitioning between the prompt and the list of
// symbols, such that we are not required to keep that as part of the running
// state. The reason why it's best this way is that the TUI is launched in a
// fixed layout that never experiments changes in size, so unless the user is
// changing their terminal emulator size, which we don't handle just yet, this
// should be constant beyond its point of initialization in `prepare_space()`.
// That is why, at the moment, we use `OnceLock`.
static PROMPT_COORD: OnceLock<(u16, u16)> = OnceLock::new();

#[tracing::instrument(skip_all, err(level = "info"))]
async fn prepare_space() -> anyhow::Result<()> {
    let mut stdout = StdBufWriter::new(std_io::stdout());

    crossterm::queue!(
        stdout,
        Print(fmt::from_fn(|f| (0..10).try_for_each(|_| writeln!(f)))),
    )?;

    info!(position_pre_saving = ?cursor::position().unwrap());

    crossterm::queue!(stdout, MoveToPreviousLine(10), SavePosition)?;

    info!(position_post_saving = ?cursor::position().unwrap());

    PROMPT_COORD.set(cursor::position()?).unwrap();

    task::block_in_place(|| stdout.flush())?;

    Ok(())
}

// TODO: if time allows, get the part of `main` that enables raw mode to also
// run here, as well as `prepare_space`. Possibly use a channel to update the
// messages that would get reported on each of the tasks.
#[tracing::instrument(skip_all, err(level = "info"))]
async fn init() -> anyhow::Result<ConstContainer> {
    info!("starting init routine");

    let mut stdout = StdBufWriter::new(std_io::stdout());

    // NOTE: we require saving the position prior to starting the spinner because
    // otherwise the task in charge of it will reset the position to the line in the
    // shell that launched the command, and not to the next line.
    task::block_in_place(|| crossterm::execute!(stdout, SavePosition))?;

    Spinner::run_while(stdout, async move |tx| {
        tx.send("Parsing `libc` repo".into())?;

        libc_constant_deprecator_lib::scan(if let Some(path) = Args::parse().path {
            PathBuf::from(path)
        } else {
            env::current_dir().unwrap()
        })
        .await
        .inspect_err(|_| info!("resolved file parsing"))
        .map_err(Into::into)
    })
    .await
}

// TODO: if time allows, try to run the program under `hyperfine` and see into
// drawing a heatmap of a test run, just to check out where can I look for
// performance gains.

// NOTE: we keep a constant with the exact bitmask we require such that popping
// off the enhancement flags from the keyboard enhancement protocol requires
// only a single event.
//
// Whether the author of the `crossterm` docs meant that the "level" that each
// push and pop operation is relevant to an abstraction of their own, or
// otherwise to an abstraction particular to the keyboard protocol remains to be
// seen, and may become a source of issues if it happens to be the latter.
//
// We avoid enabling processing of press/release events for plain text because
// that produces the raw events even when we instended on having a character
// requiring a modifier combo input.
const ENHANCEMENT_FLAGS_IN_USE: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_EVENT_TYPES);

#[defer_drm]
#[tokio::main]
#[tracing::instrument(skip_all)]
async fn main() -> anyhow::Result<()> {
    if cfg!(debug_assertions) {
        tracing_subscriber::fmt()
            .with_target(false)
            .with_level(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_thread_names(false)
            .without_time()
            .with_ansi(false)
            .with_line_number(false)
            .with_writer(StdMutex::new(
                File::create(env::current_dir().map(|pwd| pwd.join("debug.log")).unwrap()).unwrap(),
            ))
            .init();
    }

    let parsed_constants = init().await?;

    prepare_space().await?;

    task::block_in_place(|| {
        let res1 = terminal::enable_raw_mode();
        let res2 = crossterm::execute!(
            std_io::stdout(),
            PushKeyboardEnhancementFlags(ENHANCEMENT_FLAGS_IN_USE)
        );

        res1.and(res2)
    })?;

    info!(prompt_coordinates = ?PROMPT_COORD.get().unwrap());

    let (state, events_tx, msg_rx) = State::new(parsed_constants);

    // NOTE: below, we use an IO stream from `std` instead of the static that we
    // used throughout exeuction because we know for sure that at this point there's
    // no way `stdout`  is locked; If we bailed out, then the guard over the async
    // `Mutex` has been dropped and the stream can thus be used without fear.
    //
    // It's neither feasible to get a lock inside the closures used with
    // `inspect_err` because those are sync, but locking the static with `stdout`
    // requires an async context. And, of course, locking prior to even spawning
    // or completing the main tasks is not feasible because those tasks require
    // locking the static themselves.
    //
    // The only way the above reasoning is unsound is if the process panics with a
    // non-unwinding strategy, in which case it would just bail out without running
    // the corresponding destructors (thus not dropping the guard over the mutex
    // that may have been held while the thread panicked, and possibly failing to
    // write through the instance of `Stdout` we fetch within the below clsoures.)
    future::try_join(
        task::spawn(handle_input(events_tx)),
        task::spawn(render(state, msg_rx)),
    )
    .await
    .map(|(res1, res2)| res1.and(res2))
    .inspect_err(|_| {
        crossterm::execute!(std_io::stdout(), PopKeyboardEnhancementFlags).unwrap();
    })?
    .inspect_err(|_| {
        crossterm::execute!(std_io::stdout(), PopKeyboardEnhancementFlags).unwrap();
    })?;

    task::block_in_place(|| {
        crossterm::execute!(
            std_io::stdout(),
            RestorePosition,
            Clear(ClearType::FromCursorDown),
            PopKeyboardEnhancementFlags
        )
    })?;

    Ok(())
}
