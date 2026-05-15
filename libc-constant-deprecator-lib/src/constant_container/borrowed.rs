use std::{
    ops::{Bound, ControlFlow, IntoBounds, Range, Try},
    sync::Arc,
};

use tracing::{info, info_span};

use crate::{BorrowedElement, Const, Sealed, borrowed};

pub(crate) mod borrowed_element;

/// Represents a borrowed view into multiple segments of a [`ConstContainer`] as
/// a single, contiguous container of its own.
///
/// This is produced as part of [`filter()`]ing [`Const`]s in a
/// [`ConstContainer`], or by calling [`ConstContainer::borrowed()`], which
/// should yield an empty borrowed view that can be reused with
/// [`filter_with()`].
///
/// [`filter()`]: `crate::ConstContainer::filter()`
/// [`ConstContainer`]: `crate::ConstContainer`
/// [`ConstContainer::borrowed()`]: `crate::ConstContainer::borrowed()`
/// [`filter_with()`]: `crate::ConstContainer::filter_with()`
#[derive(Debug)]
pub struct BorrowedContainer {
    source: Vec<BorrowedElement>,
    init_state: Vec<bool>,
}

impl BorrowedContainer {
    pub(crate) fn from_container(container: &[Arc<(Const, bool)>]) -> Self {
        let out = Self {
            init_state: container.iter().map(|ptr| ptr.0.is_deprecated()).collect(),
            source: container
                .iter()
                .map(Arc::downgrade)
                .map(|ptr| borrowed!(ptr))
                .collect(),
        };

        if cfg!(debug_assertions) {
            let preemptive_span = info_span!("preemptive_info");

            out.init_state
                .iter()
                .zip(&out.source)
                .filter_map(|(init_state, elem)| {
                    elem.with_inner(|constant, _| {
                        constant
                            .ident()
                            .to_string()
                            .contains("MINI")
                            .then_some((constant.ident().to_string(), *init_state))
                    })
                    .flatten()
                })
                .for_each(|(ident, init_state)| {
                    info!(parent: &preemptive_span, ident, init_state = init_state);
                });
        }

        out
    }

    // NOTE: this is used in debugging builds to get raw access to all two buffers
    // of the borrowed container, as the `Visit` trait is implemented as a public
    // interface to exernal code that provides a view solely into the constant.
    #[cfg(debug_assertions)]
    pub(crate) fn traverse(&self, f: impl Fn(&Const, bool)) {
        let Self { source, init_state } = self;

        source
            .iter()
            .zip(init_state.iter().copied())
            .for_each(|(constant, init_state)| {
                constant.with_inner(|constant, _| f(constant, init_state));
            });
    }

    pub(crate) fn buffer_mut(&mut self) -> &mut [BorrowedElement] {
        &mut self.source
    }
}

impl BorrowedContainer {
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn len(&self) -> usize {
        let Self { source, .. } = self;

        source.len()
    }
}

/// Defines shared behavior between types capable of indexing into
/// [`BorrowedContainer`].
///
/// This is sealed because the underlying container type in `BorrowedContainer`
/// is not implemented in this crate, and only allows a limited range of types
/// to index into it.
#[expect(
    private_bounds,
    reason = "The whole point of the `Sealed` pattern is to not allow public implementations of \
              it."
)]
pub trait Indexer<T>: Sealed {
    fn eval(self) -> T;
}

// TODO: if time allows, write a macro to get rid of the following repetitive
// implementations.

impl<I: Into<usize>> Sealed for Range<I> {}

impl<I: Into<usize>> Sealed for &Range<I> {}

impl<I: Into<usize>> Sealed for &mut Range<I> {}

impl<I: Into<usize>> Indexer<Range<I>> for Range<I> {
    fn eval(self) -> Range<I> {
        self
    }
}

impl<I: Into<usize> + Clone> Indexer<Range<I>> for &Range<I> {
    fn eval(self) -> Range<I> {
        self.clone()
    }
}

impl<I: Into<usize> + Clone> Indexer<Range<I>> for &mut Range<I> {
    fn eval(self) -> Range<I> {
        self.clone()
    }
}

/// Represents a subset of a borrowed view (i.e. a subset of a
/// [`BorrowedContainer`].)
///
/// This is produced as part of [`select()`], which further yields a contiguous
/// subset of the superset of disjoint references into the source container. The
/// hierarchy, in decreasing levels of ownership/dependence, goes like this:
/// [`ConstContainer`] -> [`BorrowedContainer`] -> [`BorrowedSubset`].
///
/// A subset is tied to its borrowed view, which itself is tied (weakly) to the
/// owning container. The difference between a borrowed view and a subset of it
/// is thus that the borrowed view may be disjoint, while the subset is always a
/// contiguous view into the borrowed view.
///
/// [`select()`]: `crate::BorrowedContainer::select()`
/// [`ConstContainer`]: `crate::ConstContainer`
#[derive(Debug)]
pub struct BorrowedSubset<'a> {
    source: &'a mut [BorrowedElement],
    init_state: &'a [bool],
}

impl<'a> BorrowedSubset<'a> {
    fn new(source: &'a mut [BorrowedElement], init_state: &'a [bool]) -> Self {
        Self { source, init_state }
    }
}

impl BorrowedContainer {
    /// Yields a borrowed subset over the borrowed view, only valid for the
    /// lifetime of the latter.
    ///
    /// This is useful when requiring multiple subsequent levels of detail into
    /// the same overarching `ConstContainer`, without giving up on any of the
    /// intermediate, borrowed views.
    pub fn select<I: Into<usize>>(&mut self, range: impl Indexer<Range<I>>) -> BorrowedSubset<'_> {
        let Self { source, init_state } = self;
        let (start, end) = range.eval().into_bounds();

        let start = start.map(Into::into);
        let end = end.map(Into::into);

        let source = &mut source[(start, end)];
        let init_state = &mut init_state[(start, end)];

        info!(subset_range = ?Range { start, end });

        #[cfg(debug_assertions)]
        {
            source
                .iter()
                .zip(init_state.iter())
                .for_each(|(elem, init_state)| {
                    if elem
                        .with_inner(|constant, _| {
                            info!(
                                "matched constant: {constant}:{init_state}",
                                constant = constant.ident(),
                                init_state = init_state
                            );
                        })
                        .is_none()
                    {
                        info!("unmatched constant");
                    }
                });
        }

        BorrowedSubset::new(source, init_state)
    }
}

/// Utility trait to enable common constant symbol traversal pattern between
/// borrowed views.
///
/// This is implemented for both [`BorrowedContainer`] and [`BorrowedSubset`],
/// such that in-place (weaker) iteration through the constants is possible.
#[expect(
    private_bounds,
    reason = "The whole point of the `Sealed` pattern is to not allow public implementations of \
              it."
)]
pub trait Visit: Sealed {
    /// Provides an immutable traversal throughout gathered constants that can
    /// be put on halt.
    ///
    /// This is akin to a far less powerful version of iteration that gates the
    /// actual iterator, and only provides a temporary, in-place view with a
    /// closure that can capture callsite state.
    fn visit<B>(&self, visitor: impl FnMut(&Const) -> ControlFlow<B>) -> Option<B>;

    /// Provides a mutable traversal throughout gathered constants that can be
    /// put on halt.
    ///
    /// This is akin to a far less powerful version of iteration that gates the
    /// actual iterator, and only provides a temporary, in-place view with a
    /// closure that can capture callsite state.
    fn visit_mut<B>(&mut self, visitor: impl FnMut(&mut Const) -> ControlFlow<B>) -> Option<B>;

    /// Provides an immutable traversal throughout matched constants that can be
    /// put on halt.
    ///
    /// Note the method will attempt to iterate through all constants that
    /// matched the last regex with which the implementor got filled. This means
    /// that the method will either traverse the first ten (possibly disjoint)
    /// matched constants, or otherwise not traverse any constant whatsoever.
    fn visit_n<B, R: Try<Output = (), Residual = B>>(
        &self,
        n: usize,
        mut visitor: impl FnMut(&Const) -> R,
    ) -> Option<B> {
        let mut counter = 0;

        self.visit(|constant| {
            if counter == n - 1 {
                return ControlFlow::Break(None);
            }

            counter += 1;

            visitor(constant).branch().map_break(Some)
        })
        .flatten()
    }

    /// Provided an index into the collection of symbols being traversed, this
    /// routine attempts to find it and perform some operation on it.
    fn find_indexed<T>(&self, idx: impl Into<usize>, mut f: impl FnMut(&Const) -> T) -> Option<T> {
        let mut counter = 0;
        let idx = idx.into();

        self.visit(|constant| {
            if counter == idx {
                return ControlFlow::Break(f(constant));
            }

            counter += 1;

            ControlFlow::Continue(())
        })
    }

    /// Provided an identifier, this routine traverses the collection of symbols
    /// attempting to find some symbol that matches such identifier, and
    /// provides access to it.
    fn find_named<T>(&self, sym: impl AsRef<str>, mut f: impl FnMut(&Const) -> T) -> Option<T> {
        self.visit(|constant| {
            if *constant.ident() == sym {
                return ControlFlow::Break(f(constant));
            }

            ControlFlow::Continue(())
        })
    }

    // NOTE: this may be not be necessary considering there is already a `select`
    // routine for `BorrowedContainer` to take a subset of its elements into a
    // `BorrowedSubset`, itself an implementor of `Visit`.
    /// Provided a range into the collection being traversed, this routine
    /// allows running a closure over the set of constants whose traversal index
    /// is within such range.
    fn select<T, I: Into<usize>, R: Try<Output = (), Residual = T>>(
        &self,
        range: impl Indexer<Range<I>>,
        mut f: impl FnMut(&Const) -> R,
    ) -> Option<T> {
        let (start, end) = range.eval().into_bounds();
        let Bound::Included(start) = start.map(Into::into) else {
            panic!("start bound is always included below in the considered range")
        };
        let Bound::Excluded(end) = end.map(Into::into) else {
            panic!("end bound is alwasy excluded above in the considered range");
        };

        let mut counter = 0;

        self.visit(move |constant| {
            if counter < start {
                counter += 1;

                info!(constant_in_bounds = false);

                return ControlFlow::Continue(());
            } else if counter == end {
                return ControlFlow::Break(None);
            }

            info!(constant_in_bounds = true);

            if let ControlFlow::Break(res) = f(constant).branch() {
                return ControlFlow::Break(res.into());
            }

            counter += 1;

            ControlFlow::Continue(())
        })
        .flatten()
    }
}

impl Sealed for BorrowedContainer {}

impl Sealed for BorrowedSubset<'_> {}

macro_rules! visit_impl {
    (@body @iter @mut => $source:expr) => { $source.iter_mut() };
    (@body @iter @ref => $source:expr) => { $source.iter() };
    (@body @elem @mut => $elem:expr, $f:expr) => { $elem.with_inner_mut($f) };
    (@body @elem @ref => $elem:expr, $f:expr) => { $elem.with_inner($f) };
    (@body @$spec:tt => $self:expr, $visitor:expr) => {
        let Self { source, .. } = $self;

        if let ControlFlow::Break(b) = visit_impl!(@body @iter @$spec => source).try_for_each(|elem| {
            if let Some(res) = visit_impl!(@body @elem @$spec => elem, |constant, _| $visitor(constant))
                && res.is_break()
            {
                res
            } else {
                ControlFlow::Continue(())
            }
        }) {
            b.into()
        } else {
            None
        }
    };
    (@proto) => {
        fn visit<B>(&self, mut visitor: impl FnMut(&Const) -> ControlFlow<B, ()>) -> Option<B> {
            visit_impl! { @body @ref => self, visitor }
        }

        fn visit_mut<B>(
            &mut self,
            mut visitor: impl FnMut(&mut Const) -> ControlFlow<B, ()>,
        ) -> Option<B> {
            visit_impl! { @body @mut => self, visitor }
        }
    };
    () => {
        impl Visit for BorrowedContainer {
            visit_impl! { @proto }
        }

        impl Visit for BorrowedSubset<'_> {
            visit_impl! { @proto }
        }
    };
}

visit_impl! {}

macro_rules! deprecate_impl {
    (@body @deprecate) => { true };
    (@body @undeprecate) => { false };
    (@body $op:tt, $self:expr) => {
        use tracing::{info, info_span};

        // NOTE: Yes, it's odd that type inference does not work here, but apparently it
        // only works when destructuring the overaching `BorrowedContainer`, which
        // doesn't work as well when destrucuring the `BorrowedSubset`, because the
        // latter holds references into the former, and so you end up with further
        // indirection. This then makes the logic that follows not work for both types.
        let source: &mut [BorrowedElement] = $self.source.as_mut();
        let init_state: &[bool] = $self.init_state.as_ref();

        if cfg!(debug_assertions) {
            let preemptive_span = info_span!("preemptive_info");

            source
                .iter()
                .zip(init_state)
                .for_each(|(elem, init_state)| {
                    elem.with_inner(|constant, _| {
                        info!(
                            parent: &preemptive_span,
                            ident = %constant.ident(),
                            deprecated = constant.is_deprecated(),
                            init_state,
                        );
                    });
                });
        }

        source
            .iter_mut()
            .zip(init_state)
            .for_each(|(elem, init_state)| {
                elem.with_inner_mut(|constant, modified| {
                    constant.deprecate(deprecate_impl!(@body @$op));

                    info!(
                        init_state,
                        current_state = constant.is_deprecated(),
                        constant = %constant.ident(),
                    );

                    *modified = *init_state != constant.is_deprecated();

                    info!(modified_constant = modified, deprecated = constant.is_deprecated());
                });
            });
    };
    (@doc $op:tt { $it:item }) => {
    /// Bulk
    #[doc = stringify! { $op }]
    /// all [`Const`]s gathered from the underlying [`ConstContainer`].
    ///
    /// This will mark all constants as having been modified, so long as their
    /// state by the time the borrowed container is dropped differs from that
    /// with which they entered it.
    ///
    /// [`ConstContainer`]: `crate::ConstContainer`
    $it
    };
    () => {
        impl BorrowedContainer {
            deprecate_impl! { @doc deprecate {
                #[cfg_attr(
                    debug_assertions,
                    tracing::instrument(skip_all, fields(deprecate_in_borrowed_container = true))
                )]
                pub fn deprecate(&mut self) { deprecate_impl! { @body deprecate, self } }
            } }

            deprecate_impl! { @doc undeprecate {
                #[cfg_attr(
                    debug_assertions,
                    tracing::instrument(skip_all, fields(deprecate_in_borrowed_container = true))
                )]
                pub fn undeprecate(&mut self) { deprecate_impl! { @body undeprecate, self } }
            } }
        }

        impl BorrowedSubset<'_> {
            deprecate_impl! { @doc deprecate {
                #[cfg_attr(
                    debug_assertions,
                    tracing::instrument(skip_all, fields(deprecate_in_borrowed_container = false))
                )]
                pub fn deprecate(&mut self) { deprecate_impl! { @body deprecate, self } }
            } }

            deprecate_impl! { @doc undeprecate {
                #[cfg_attr(
                    debug_assertions,
                    tracing::instrument(skip_all, fields(deprecate_in_borrowed_container = false))
                )]
                pub fn undeprecate(&mut self) { deprecate_impl! { @body undeprecate, self } }
            } }
        }
    };
}

deprecate_impl! {}
