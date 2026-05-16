use std::sync::{Arc, Weak};

use crate::Const;

macro_rules! with_inner_impl {
    (@mut => $repr:expr, $f:expr) => {
        $repr.with_inner_mut($f)
    };
    (@ref => $repr:expr, $f:expr) => {
        $repr.with_inner($f)
    };
    (@body @ $spec:tt => $self:expr, $f:expr) => {
        let Self { repr } = $self;

        with_inner_impl! { @$spec => repr, $f }
    };
    () => {
        pub(crate) fn with_inner<T>(&self, f: impl FnMut(&Const, bool) -> T) -> Option<T> {
            with_inner_impl! { @body @ref => self, f }
        }

        pub(crate) fn with_inner_mut<T>(
            &mut self,
            f: impl FnMut(&mut Const, &mut bool) -> T,
        ) -> Option<T> {
            with_inner_impl! { @body @mut => self, f }
        }
    };
}

#[derive(Debug)]
pub(crate) struct BorrowedElement {
    repr: BorrowedElementRepr,
}

impl BorrowedElement {
    pub(crate) fn new(repr: Option<Weak<(Const, bool)>>) -> Self {
        Self {
            repr: BorrowedElementRepr::new(repr),
        }
    }

    with_inner_impl! {}
}

macro_rules! with_inner_repr_impl {
    (@mut => $ptr:expr) => {
        $ptr.cast_mut().as_mut_unchecked()
    };
    (@ref => $ptr:expr) => {
        $ptr.as_ref_unchecked()
    };
    (@mut @ret => $state:expr) => {
        $state
    };
    (@ref @ret => $state:expr) => {
        *$state
    };
    (@body @$spec:tt => $self:expr, $f:expr) => {
        let Self { inner } = $self;

        if let Some(ptr) = inner
            && let Some((constant, state)) = ptr.upgrade().map(|ptr| {
                let (constant, state) = unsafe {
                    with_inner_repr_impl!(@$spec => Arc::as_ptr(&ptr))
                };

                (constant, with_inner_repr_impl! { @$spec @ret => state })
            })
        {
            $f(constant, state).into()
        } else {
            None
        }
    };
    () => {
        fn with_inner_mut<T>(&self, mut f: impl FnMut(&mut Const, &mut bool) -> T) -> Option<T> {
            with_inner_repr_impl! { @body @mut => self, f }
        }

        fn with_inner<T>(&self, mut f: impl FnMut(&Const, bool) -> T) -> Option<T> {
            with_inner_repr_impl! { @body @ref => self, f }
        }
    };
}

#[derive(Debug)]
struct BorrowedElementRepr {
    inner: Option<Weak<(Const, bool)>>,
}

impl BorrowedElementRepr {
    fn new(inner: Option<Weak<(Const, bool)>>) -> Self {
        Self { inner }
    }

    with_inner_repr_impl! {}
}
