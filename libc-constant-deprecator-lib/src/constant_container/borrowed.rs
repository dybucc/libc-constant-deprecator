use std::sync::{Arc, Weak};

use crate::Const;

/// Represents a borrowed view into multiple segments of a [`ConstContainer`] as
/// a single, contiguous container of its own.
///
/// This is produced as part of [`filter()`]ing [`Const`]s in a
/// [`ConstContainer`], or by calling [`ConstContainer::borrowed_container()`],
/// which should yield an empty borrowed view that can be reused with
/// [`filter_with()`].
///
/// [`filter()`]: `crate::ConstContainer::filter()`
/// [`ConstContainer`]: `crate::ConstContainer`
/// [`ConstContainer::borrowed_container()`]: `crate::ConstContainer::borrowed_container()`
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

macro_rules! deprecate_impl {
    (body @deprecate) => { true };
    (body @undeprecate) => { false };
    (@body $op:tt, $self:expr) => {
        let Self { source, init_state } = $self;

        source
            .iter_mut()
            .map(|ptr| ptr.upgrade().map(|ptr| unsafe { Arc::as_ptr(&ptr).cast_mut().as_mut_unchecked() }))
            .filter_map(|ptr| ptr)
            .zip(init_state)
            .for_each(|((constant, modified), init_modified)| {
                constant.deprecated(deprecate_impl!(body @$op));

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
    };
}

deprecate_impl! {}
