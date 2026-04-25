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

        for (constant, modified) in inner {
            constant.deprecated(true);
            *modified = true;
        }
    }

    pub fn undeprecate(&mut self) {
        let BorrowedContainer(inner) = self;
        for (constant, modified) in inner {
            constant.deprecated(false);
            *modified = true;
        }
    }
}
