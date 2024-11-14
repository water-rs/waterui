use core::hash::Hash;

pub trait Identifable {
    type Id: Hash + Ord;
    fn id(&self) -> Self::Id;
}
#[derive(Debug)]
pub struct UseId<T, F> {
    value: T,
    f: F,
}

impl<T, F, Id> Identifable for UseId<T, F>
where
    F: Fn(&T) -> Id,
    Id: Ord + Hash,
{
    type Id = Id;
    fn id(&self) -> Self::Id {
        (self.f)(&self.value)
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SelfId<T>(T);

impl<T> SelfId<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T: Hash + Ord + Clone> Identifable for SelfId<T> {
    type Id = T;
    fn id(&self) -> Self::Id {
        self.0.clone()
    }
}

pub trait IdentifableExt: Sized {
    fn use_id<F>(self, f: F) -> UseId<Self, F>;
    fn self_id(self) -> SelfId<Self>;
}

impl<T> IdentifableExt for T {
    fn use_id<F>(self, f: F) -> UseId<Self, F> {
        UseId { value: self, f }
    }
    fn self_id(self) -> SelfId<Self> {
        SelfId(self)
    }
}
