use std::{
    ops::{Bound, ControlFlow, IntoBounds, Range, Try},
    sync::{Arc, Weak},
};

use tracing::info;

use crate::{Const, Sealed};

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
    source: Vec<Weak<(Const, bool)>>,
    init_state: Vec<bool>,
}

impl BorrowedContainer {
    pub(crate) fn from_container(container: &[Arc<(Const, bool)>]) -> Self {
        Self {
            init_state: container.iter().map(|ptr| ptr.1).collect(),
            source: container.iter().map(Arc::downgrade).collect(),
        }
    }

    pub(crate) fn buffer(&mut self) -> &mut Vec<Weak<(Const, bool)>> {
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

impl<I: Into<usize>> Sealed for Range<I> {}

impl<I: Into<usize>> Sealed for &Range<I> {}

impl<I: Into<usize>> Sealed for &mut Range<I> {}

// TODO: if time allows, write a macro to get rid of the following repetitive
// implementations.

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
    source: &'a mut [Weak<(Const, bool)>],
    init_state: &'a [bool],
}

impl<'a> BorrowedSubset<'a> {
    fn new(source: &'a mut [Weak<(Const, bool)>], init_state: &'a [bool]) -> Self {
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

        BorrowedSubset::new(&mut source[(start, end)], &init_state[(start, end)])
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
    fn visit<B>(&self, visitor: impl FnMut(&Const) -> ControlFlow<B, ()>) -> Option<B>;

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

    /// Provided a range into the collection being traversed, this routine
    /// allows running a closure over the set of constants whose traversal index
    /// is within such range.
    fn select<T, I: Into<usize>, R: Try<Output = (), Residual = T>>(
        &self,
        range: impl Indexer<Range<I>>,
        mut f: impl FnMut(&Const) -> R,
    ) -> Option<T> {
        let (start, end) = range.eval().into_bounds();
        let start = start.map(Into::into);
        let end = end.map(Into::into);

        let mut counter = 0;

        self.visit(move |constant| {
            if counter < {
                let Bound::Included(start) = start else {
                    panic!("start bound is always included below in the considered range")
                };

                info!(contant_in_bounds = false);

                start
            } {
                counter += 1;

                return ControlFlow::Continue(());
            } else if counter == {
                let Bound::Excluded(end) = end else {
                    panic!("end bound is alwasy excluded above in the considered range");
                };

                end
            } {
                return ControlFlow::Break(None);
            }

            info!(contant_in_bounds = true);

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
    (@body) => {
        fn visit<B>(&self, visitor: impl FnMut(&Const) -> ControlFlow<B, ()>) -> Option<B> {
            let Self { source, .. } = self;

            if let ControlFlow::Break(b) = source
                .iter()
                .map(Weak::upgrade)
                .filter_map(|ptr| ptr.map(|ptr| unsafe { &Arc::as_ptr(&ptr).as_ref_unchecked().0 }))
                .try_for_each(visitor)
            {
                b.into()
            } else {
                None
            }
        }
    };
    () => {
        impl Visit for BorrowedContainer {
            visit_impl! { @body }
        }

        impl Visit for BorrowedSubset<'_> {
            visit_impl! { @body }
        }
    };
}

visit_impl! {}

macro_rules! deprecate_impl {
    (body @deprecate) => { true };
    (body @undeprecate) => { false };
    (@body $op:tt, $self:expr) => {
        use tracing::info;

        // NOTE: Yes, it's odd that type inference does not work here, but apparently it
        // only works when destructuring the overaching `BorrowedContainer`, which
        // doesn't work as well when destrucuring the `BorrowedSubset`, because the
        // latter holds references into the former, and so you end up with further
        // indirection. This then makes the logic that follows not work for both types.
        let source: &mut [Weak<(Const, bool)>] = $self.source.as_mut();
        let init_state: &[bool] = $self.init_state.as_ref();

        source
            .iter_mut()
            .filter_map(|ptr| {
                ptr.upgrade().map(|ptr| unsafe {
                    Arc::as_ptr(&ptr).cast_mut().as_mut_unchecked()
                })
            })
            .zip(init_state)
            .for_each(|((constant, modified), init_modified)| {
                info!(modified_constant = true, deprecated = deprecate_impl!(body @$op));

                constant.deprecate(deprecate_impl!(body @$op));

                *modified = *init_modified == *modified;
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
                #[cfg_attr(debug_assertions, tracing::instrument(skip_all))]
                pub fn deprecate(&mut self) { deprecate_impl! { @body deprecate, self } }
            } }

            deprecate_impl! { @doc undeprecate {
                #[cfg_attr(debug_assertions, tracing::instrument(skip_all))]
                pub fn undeprecate(&mut self) { deprecate_impl! { @body undeprecate, self } }
            } }
        }

        impl BorrowedSubset<'_> {
            deprecate_impl! { @doc deprecate {
                #[cfg_attr(debug_assertions, tracing::instrument(skip_all))]
                pub fn deprecate(&mut self) { deprecate_impl! { @body deprecate, self } }
            } }

            deprecate_impl! { @doc undeprecate {
                #[cfg_attr(debug_assertions, tracing::instrument(skip_all))]
                pub fn undeprecate(&mut self) { deprecate_impl! { @body undeprecate, self } }
            } }
        }
    };
}

deprecate_impl! {}
