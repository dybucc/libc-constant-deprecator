use crate::{Const, ConstContainer};

// NOTE: this container serves as a builder for `ConstContainer`, such that it
// adds up different instances of itself, which matches the pattern with which
// constants are parsed (producing results on a single-file basis for each file
// of the `libc` codebase.)

#[derive(Debug)]
pub(crate) struct ConstContainerBuilder {
    repr: Vec<Const>,
}

impl FromIterator<Const> for ConstContainerBuilder {
    fn from_iter<T: IntoIterator<Item = Const>>(iter: T) -> Self {
        Self {
            repr: iter.into_iter().collect(),
        }
    }
}

impl ConstContainerBuilder {
    pub(crate) fn new() -> Self {
        Self { repr: Vec::new() }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        let Self { repr } = self;
        let Self { repr: orepr } = other;

        repr.extend(orepr);
    }

    pub(crate) fn finish(self) -> ConstContainer {
        let Self { repr } = self;

        ConstContainer::new(repr)
    }
}
