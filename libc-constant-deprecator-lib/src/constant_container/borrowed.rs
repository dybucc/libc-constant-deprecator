use std::{
    ops::{ControlFlow, Range},
    sync::{Arc, Weak},
};

use crate::Const;

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

// NOTE: we use a trait here to define shared behavior between taking an owning
// range, or a reference to one, but it may just be better to have three
// different implementations of `Index` packed in a macro.
trait Indexer<T> {
    fn eval(self) -> T;
}

impl Indexer<Range<usize>> for Range<usize> {
    fn eval(self) -> Range<usize> {
        self
    }
}

impl Indexer<Range<usize>> for &Range<usize> {
    fn eval(self) -> Range<usize> {
        self.clone()
    }
}

impl Indexer<Range<usize>> for &mut Range<usize> {
    fn eval(self) -> Range<usize> {
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
    #[expect(
        private_bounds,
        reason = "It's meant to be this way. The trait with which we can index into the borrowed \
                  view is not meant to be implementable by library users."
    )]
    pub fn select(&mut self, range: impl Indexer<Range<usize>>) -> BorrowedSubset<'_> {
        let Self { source, init_state } = self;
        let selected = range.eval();

        BorrowedSubset::new(&mut source[selected.clone()], &init_state[selected])
    }
}

/// Utility trait to enable common constant symbol traversal pattern between
/// borrowed views.
///
/// This is implemented for both [`BorrowedContainer`] and [`BorrowedSubset`],
/// such that in-place (weaker) iteration through the constants is possible.
pub trait Visit {
    /// Provides an immutable traversal throughout gathered constants that can
    /// be put on halt.
    ///
    /// This is akin to a far less powerful version of iteration that gates the
    /// actual iterator, and only provides a temporary, in-place view with a
    /// closure that can capture callsite state.
    fn visit<B>(&self, visitor: impl FnMut(&Const) -> ControlFlow<B, ()>) -> Option<B>;
}

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
        // NOTE: Yes, it's odd that type inference does not work here, but apparently it
        // only works when destructuring the overaching `BorrowedContainer`, which
        // doesn't work as well when destrucuring the `BorrowedSubset`, because the
        // latter holds references into the former, and so you end up with further
        // indirection. This then makes the logic that follows not work for both types.
        let source: &mut [Weak<(Const, bool)>] = $self.source.as_mut();
        let init_state: &[bool] = $self.init_state.as_ref();

        source
            .iter_mut()
            .filter_map(|ptr| ptr.upgrade().map(|ptr| unsafe {
                Arc::as_ptr(&ptr).cast_mut().as_mut_unchecked()
            }))
            .zip(init_state)
            .for_each(|((constant, modified), init_modified)| {
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
                pub fn deprecate(&mut self) { deprecate_impl! { @body deprecate, self } }
            } }

            deprecate_impl! { @doc undeprecate {
                pub fn undeprecate(&mut self) { deprecate_impl! { @body undeprecate, self } }
            } }
        }

        impl BorrowedSubset<'_> {
            deprecate_impl! { @doc deprecate {
                pub fn deprecate(&mut self) { deprecate_impl! { @body deprecate, self } }
            } }

            deprecate_impl! { @doc undeprecate {
                pub fn undeprecate(&mut self) { deprecate_impl! { @body undeprecate, self } }
            } }
        }
    };
}

deprecate_impl! {}
