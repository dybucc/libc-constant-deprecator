use crate::Const;

pub struct BorrowedContainer<'a>(pub(crate) Vec<&'a mut (Const, bool)>);

impl<'a> BorrowedContainer<'a> {
    pub(crate) fn new(container: Vec<&'a mut (Const, bool)>) -> Self {
        Self(container)
    }
}

impl BorrowedContainer<'_> {
    pub fn deprecate(&mut self) {
        let BorrowedContainer(inner) = self;

        inner
            .iter_mut()
            .filter_map(|(constant, modified)| if *modified { constant.into() } else { None })
            .for_each(|constant| constant.deprecated(true));
    }
}
