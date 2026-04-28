use crate::Const;

/// Represents a borrowed view into multiple segments of a [`ConstContainer`] as
/// a single, contiguous container of its own.
///
/// This is produced as part of [`filter()`]ing [`Const`]s in a
/// [`ConstContainer`].
///
/// [`filter()`]: `ConstContainer::filter()`
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

impl BorrowedContainer<'_> {
    /// Bulk deprecate all [`Const`]s gathered from the underlying
    /// [`ConstContainer`].
    ///
    /// This will mark all constants as having been modified, so long as their
    /// state by the time borrowed container is dropped differs from that with
    /// which they entered the exclusive view container.
    pub fn deprecate(&mut self) {
        let BorrowedContainer { source, init_state } = self;

        source
            .iter_mut()
            .zip(init_state)
            .for_each(|((constant, modified), init_modified)| {
                constant.deprecated(true);

                // NOTE: this ensures that we really only mark as modified those items in the
                // overarching `ConstContainer` that have been modified from the point in which
                // the borrowed view got created.
                //
                // At this point, if `modified` is equivalent to `init_modified`, then surely we
                // will be toggling the state to its dual equivalent, so the flag ought be
                // toggled as well. Otherwise, we are coming in from having already toggled the
                // state previously (which made them unequivalent,) and so the flag ought be
                // restored because the state following is the one equivalent to the one with
                // which `init_modified` got introduced in the borrowed view.
                *modified = *init_modified == *modified;
            });
    }

    /// Bulk undeprecate all [`Const`]s gathered from the underlying
    /// [`ConstContainer`].
    ///
    /// This will mark all constants as having been modified, so long as their
    /// state by the time borrowed container is dropped differs from that with
    /// which they entered the exclusive view container.
    pub fn undeprecate(&mut self) {
        let BorrowedContainer { source, init_state } = self;

        source
            .iter_mut()
            .zip(init_state)
            .for_each(|((constant, modified), init_modified)| {
                constant.deprecated(false);

                // NOTE: this ensures that we really only mark as modified those items in the
                // overarching `ConstContainer` that have been modified from the point in which
                // the borrowed view got created.
                //
                // At this point, if `modified` is equivalent to `init_modified`, then surely we
                // will be toggling the state to its dual equivalent, so the flag ought be
                // toggled as well. Otherwise, we are coming in from having already toggled the
                // state previously (which made them unequivalent,) and so the flag ought be
                // restored because the state following is the one equivalent to the one with
                // which `init_modified` got introduced in the borrowed view.
                *modified = *init_modified == *modified;
            });
    }
}
