use crate::Const;

/// Represents a borrowed view into multiple segments of a [`ConstContainer`] as
/// a single, contiguous container of its own.
///
/// This is produced as part of [`filter()`]ing [`Const`]s in a
/// [`ConstContainer`].
///
/// [`filter()`]: `crate::ConstContainer::filter()`
/// [`ConstContainer`]: `crate::ConstContainer`
#[derive(Debug)]
pub struct BorrowedContainer<'a> {
    pub(crate) source: Vec<&'a mut (Const, bool)>,
    pub(crate) init_state: Vec<bool>,
}

impl<'a> BorrowedContainer<'a> {
    pub(crate) fn new(container: Vec<&'a mut (Const, bool)>) -> Self {
        Self {
            init_state: container
                .iter()
                .map(|&&mut (_, modified)| modified)
                .collect(),
            source: container,
        }
    }
}

macro_rules! deprecate_impl {
    (body @deprecate) => { true };
    (body @undeprecate) => { false };
    (@body $op:tt, $self:expr) => {
        let BorrowedContainer { source, init_state } = $self;

        source
            .iter_mut()
            .zip(init_state)
            .for_each(|((constant, modified), init_modified)| {
                constant.deprecated(deprecate_impl!(body @$op));

                *modified = *init_modified == *modified;
            });
    };
    (doc @deprecate) => { "deprecate" };
    (doc @undeprecate) => { "undeprecate" };
    (@doc $op:tt { $it:item }) => {
    /// Bulk
    #[doc = deprecate_impl! { doc @$op }]
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
        impl BorrowedContainer<'_> {
            deprecate_impl! { @doc deprecate {
                pub fn deprecate(&mut self) { deprecate_impl! { @body deprecate, self } }
            } }

            deprecate_impl! { @doc undeprecate {
                pub fn undeprecate(&mut self) { deprecate_impl! { @body undeprecate, self } }
            } }
        }
    };
}

deprecate_impl!();
