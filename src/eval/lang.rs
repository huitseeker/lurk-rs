use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;

use bellperson::Circuit;

use crate::coprocessor::Coprocessor;
use crate::field::LurkField;
use crate::store::{Ptr, Store};
use crate::sym::Sym;

// TODO: Define a trait for the Hash and parameterize on that also.
#[derive(Debug, Clone)]
pub struct Lang<'a, F: LurkField> {
    coprocessors: HashMap<Sym, Box<&'a dyn Coprocessor<F>>>,
}

impl<'a, F: LurkField> Lang<'a, F> {
    pub fn new() -> Self {
        Self {
            coprocessors: Default::default(),
        }
    }

    pub fn add_coprocessor(&mut self, name: Sym, cproc: &'a dyn Coprocessor<F>) {
        self.coprocessors.insert(name, Box::new(cproc));
    }

    pub fn lookup(&self, s: &Store<F>, name: Ptr<F>) -> Option<&dyn Coprocessor<F>> {
        let name_ptr = s.fetch_maybe_sym(&name);

        name_ptr
            .as_ref()
            .and_then(|sym| self.coprocessors.get(sym))
            .map(|x| **x)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::coprocessor::DumbCoprocessor;
    use crate::store::Store;

    use pasta_curves::pallas::Scalar as Fr;

    #[test]
    fn lang() {
        Lang::<Fr>::new();
    }

    #[test]
    fn dumb_lang() {
        let mut lang = Lang::<Fr>::new();
        let name = Sym::new(".cproc.dumb".to_string());
        let dumb = DumbCoprocessor::new();

        lang.add_coprocessor(name, &dumb);
    }
}
