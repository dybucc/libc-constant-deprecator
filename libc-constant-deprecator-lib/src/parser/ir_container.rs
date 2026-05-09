use crate::{Const, ConstContainer};

#[derive(Debug)]
pub(crate) struct IrContainer {
    repr: Vec<Const>,
}

impl FromIterator<Const> for IrContainer {
    fn from_iter<T: IntoIterator<Item = Const>>(iter: T) -> Self {
        Self { repr: iter.into_iter().collect() }
    }
}

impl IrContainer {
    pub(crate) fn new() -> Self {
        Self {
            repr: Vec::new(),
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        let Self { repr } = self;
        let Self { repr: orepr } = other;

        repr.extend(orepr);
    }

    pub(crate) fn into_const_container(self) -> ConstContainer {
        let Self { repr } = self;

        ConstContainer::new(repr)
    }
}
