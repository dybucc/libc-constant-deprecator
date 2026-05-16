use std::{
    ops::DerefMut,
    sync::{Arc, Mutex},
};

use syn::ItemConst;

#[derive(Debug)]
pub(crate) struct ItemType {
    repr: Arc<Mutex<ItemConst>>,
}

impl ItemType {
    pub(super) fn new(item: &Arc<Mutex<ItemConst>>) -> Self {
        Self {
            repr: Arc::clone(item),
        }
    }

    #[track_caller]
    pub(crate) fn get(&mut self) -> impl DerefMut<Target = ItemConst> {
        let ItemType { repr } = self;

        repr.lock().unwrap()
    }
}
