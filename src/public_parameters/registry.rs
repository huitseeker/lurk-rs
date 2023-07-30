use std::{
    collections::{hash_map::Entry, HashMap},
    sync::{Arc, Mutex},
};

use abomonation::{decode, Abomonation};
use log::info;
use nova::traits::Group;
use once_cell::sync::Lazy;
use tap::TapFallible;

use crate::{
    coprocessor::Coprocessor,
    eval::lang::Lang,
    proof::nova::{PublicParams, G1, G2},
};
use crate::{proof::nova::CurveCycleEquipped, public_parameters::error::Error};

use super::file_map::FileIndex;

type AnyMap = anymap::Map<dyn core::any::Any + Send + Sync>;
type PublicParamMemCache<F, C> = HashMap<(usize, bool), Arc<PublicParams<'static, F, C>>>;

/// This is a global registry for Coproc-specific parameters.
/// It is used to cache parameters for each Coproc, so that they are not
/// re-initialized on each call to `eval`.
/// The use of AnyMap is a workaround for the fact that we need static storage for generic parameters,
/// noting that this is not possible in Rust.
#[derive(Clone)]
pub(crate) struct Registry {
    registry: Arc<Mutex<AnyMap>>,
}

pub(crate) static CACHE_REG: Lazy<Registry> = Lazy::new(|| Registry {
    registry: Arc::new(Mutex::new(AnyMap::new())),
});

impl Registry {
    fn get_from_file_cache_or_update_with<
        F: CurveCycleEquipped,
        C: Coprocessor<F> + 'static,
        Fn: FnOnce(Arc<Lang<F, C>>) -> Arc<PublicParams<'static, F, C>>,
    >(
        &'static self,
        rc: usize,
        abomonated: bool,
        default: Fn,
        lang: Arc<Lang<F, C>>,
    ) -> Result<Arc<PublicParams<'static, F, C>>, Error>
    where
        <<G1<F> as Group>::Scalar as ff::PrimeField>::Repr: Abomonation,
        <<G2<F> as Group>::Scalar as ff::PrimeField>::Repr: Abomonation,
    {
        // subdirectory search
        let disk_cache = FileIndex::new("public_params").unwrap();
        // use the cached language key
        let lang_key = lang.key();
        let quick_suffix = if abomonated { "-abomonated" } else { "" };
        // Sanity-check: we're about to use a lang-dependent disk cache, which should be specialized
        // for this lang/coprocessor.
        let key = format!("public-params-rc-{rc}-coproc-{lang_key}{quick_suffix}");
        // read the file if it exists, otherwise initialize
        if abomonated {
            match disk_cache.get_raw_bytes(&key) {
                Ok(mut bytes) => {
                    info!("Using abomonated public params for lang {lang_key}");
                    let (pp, rest) =
                        unsafe { decode::<PublicParams<'_, F, C>>(&mut bytes).unwrap() };
                    assert!(rest.is_empty());
                    Ok(Arc::new(pp.clone())) // this clone is VERY expensive
                }
                Err(e) => {
                    eprintln!("{e}");
                    let pp = default(lang);
                    // maybe just directly write
                    disk_cache
                        .set_abomonated(&key, &*pp)
                        .tap_ok(|_| info!("Writing public params to disk-cache: {}", lang_key))
                        .map_err(|e| Error::CacheError(format!("Disk write error: {e}")))?;
                    Ok(pp)
                }
            }
        } else {
            // read the file if it exists, otherwise initialize
            if let Some(pp) = disk_cache.get::<PublicParams<'static, F, C>>(&key) {
                info!("Using disk-cached public params for lang {lang_key}");
                Ok(Arc::new(pp))
            } else {
                let pp = default(lang);
                disk_cache
                    .set(&key, &*pp)
                    .tap_ok(|_| info!("Writing public params to disk-cache: {}", lang_key))
                    .map_err(|e| Error::CacheError(format!("Disk write error: {e}")))?;
                Ok(pp)
            }
        }
    }

    /// Check if params for this Coproc are in registry, if so, return them.
    /// Otherwise, initialize with the passed in function.
    pub(crate) fn get_coprocessor_or_update_with<
        F: CurveCycleEquipped,
        C: Coprocessor<F> + 'static,
        Fn: FnOnce(Arc<Lang<F, C>>) -> Arc<PublicParams<'static, F, C>>,
    >(
        &'static self,
        rc: usize,
        abomonated: bool,
        default: Fn,
        lang: Arc<Lang<F, C>>,
    ) -> Result<Arc<PublicParams<'static, F, C>>, Error>
    where
        F::CK1: Sync + Send,
        F::CK2: Sync + Send,
        <<G1<F> as Group>::Scalar as ff::PrimeField>::Repr: Abomonation,
        <<G2<F> as Group>::Scalar as ff::PrimeField>::Repr: Abomonation,
    {
        // re-grab the lock
        let mut registry = self.registry.lock().unwrap();
        // retrieve the per-Coproc public param table
        let entry = registry.entry::<PublicParamMemCache<F, C>>();
        // deduce the map and populate it if needed
        let param_entry = entry.or_insert_with(HashMap::new);
        match param_entry.entry((rc, abomonated)) {
            Entry::Occupied(o) => Ok(o.into_mut()),
            Entry::Vacant(v) => {
                let val = self.get_from_file_cache_or_update_with(rc, abomonated, default, lang)?;
                Ok(v.insert(val))
            }
        }
        .cloned() // this clone is VERY expensive
    }
}
