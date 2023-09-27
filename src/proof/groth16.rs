use bellpepper_core::SynthesisError;
#[cfg(not(target_arch = "wasm32"))]
use bellperson::groth16::aggregate::setup_fake_srs;
use bellperson::groth16::{
    self,
    aggregate::{
        aggregate_proofs_and_instances, verify_aggregate_proof_and_aggregate_instances,
        AggregateProofAndInstance, AggregateVersion, GenericSRS, VerifierSRS,
    },
    verify_proof,
};
use blstrs::{Bls12, Scalar};
#[cfg(not(target_arch = "wasm32"))]
use memmap::MmapOptions;
#[cfg(not(target_arch = "wasm32"))]
use once_cell::sync::Lazy;
use pairing::{Engine, MultiMillerLoop};
use rand_core::{RngCore, SeedableRng};
use rand_xorshift::XorShiftRng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::circuit::MultiFrame;
use crate::coprocessor::Coprocessor;
use crate::error::ProofError;
use crate::eval::{lang::Lang, Meta, IO};
use crate::field::LurkField;
use crate::proof::{supernova::FoldingConfig, Provable, Prover, PublicParameters};
use crate::ptr::Ptr;
use crate::store::Store;

use std::marker::PhantomData;
#[cfg(not(target_arch = "wasm32"))]
use std::{env, fs::File, io};

use super::MultiFrameTrait;

const DUMMY_RNG_SEED: [u8; 16] = [
    0x01, 0x03, 0x02, 0x04, 0x05, 0x07, 0x06, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0C, 0x0B, 0x0A,
];

/// The SRS for the inner product argument.
#[cfg(not(target_arch = "wasm32"))]
pub static INNER_PRODUCT_SRS: Lazy<GenericSRS<Bls12>> = Lazy::new(|| load_srs().unwrap());

#[cfg(not(target_arch = "wasm32"))]
const MAX_FAKE_SRS_SIZE: usize = (2 << 14) + 1;

/// A domain separator for the transcript.
pub const TRANSCRIPT_INCLUDE: &[u8] = b"LURK-CIRCUIT";

// If you don't have a real SnarkPack SRS symlinked, generate a fake one.
// Don't use this in production!
#[cfg(not(target_arch = "wasm32"))]
const FALLBACK_TO_FAKE_SRS: bool = true;

#[cfg(not(target_arch = "wasm32"))]
fn load_srs() -> Result<GenericSRS<Bls12>, io::Error> {
    let path = env::current_dir()?.join("params/v28-fil-inner-product-v1.srs");
    let f = File::open(path);

    match f {
        Ok(f) => {
            let srs_map = unsafe { MmapOptions::new().map(&f)? };
            GenericSRS::read_mmap(&srs_map, MAX_FAKE_SRS_SIZE)
        }
        Err(e) => {
            let mut rng = XorShiftRng::from_seed(DUMMY_RNG_SEED);

            if FALLBACK_TO_FAKE_SRS {
                Ok(setup_fake_srs::<Bls12, _>(&mut rng, MAX_FAKE_SRS_SIZE))
            } else {
                Err(e)
            }
        }
    }
}

/// A struct representing a proof using the Groth16 proving system with the specified engine.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proof<E: Engine + MultiMillerLoop>
where
    <E as Engine>::Gt: blstrs::Compress + Serialize,
    <E as Engine>::G1: Serialize,
    <E as Engine>::G1Affine: Serialize,
    <E as Engine>::G2Affine: Serialize,
    <E as Engine>::Fr: Serialize + LurkField,
    <E as Engine>::Gt: blstrs::Compress + Serialize,
{
    /// The aggregate proof and instance.
    #[serde(bound(
        serialize = "AggregateProofAndInstance<E>: Serialize",
        deserialize = "AggregateProofAndInstance<E>: Deserialize<'de>"
    ))]
    pub proof: AggregateProofAndInstance<E>,
    /// The number of proofs in the aggregate proof.
    pub proof_count: usize,
    /// The number of reductions used in the proof.
    pub reduction_count: usize,
}

impl<'a, C: Coprocessor<Scalar> + 'a, M: MultiFrameTrait<'a, Scalar, C>>
    Groth16Prover<'a, Bls12, C, Scalar, M>
{
    /// Creates Groth16 parameters using the given reduction count.
    pub fn create_groth_params(
        reduction_count: usize,
        lang: Arc<Lang<Scalar, C>>,
    ) -> Result<PublicParams<Bls12>, SynthesisError> {
        let multiframe: MultiFrame<'_, Scalar, C> = MultiFrame::blank(
            Arc::new(FoldingConfig::new_ivc(lang, reduction_count)),
            Meta::Lurk,
        );

        // WARNING: These parameters are totally bogus. Real Groth16 parameters need to be
        // generated by a trusted setup. We create them *deterministically* from a seeded RNG
        // so that multiple runs will create the same 'random' parameters.
        // If you use these parameters in production, anyone can make fake proofs.
        let rng = &mut XorShiftRng::from_seed(DUMMY_RNG_SEED);
        let params = groth16::generate_random_parameters::<Bls12, _, _>(multiframe, rng)?;
        Ok(PublicParams(params))
    }

    /// Generates a Groth16 proof using the given multi_frame, parameters, and random number generator.
    pub fn prove<R: RngCore>(
        &self,
        multi_frame: MultiFrame<'_, Scalar, C>,
        params: &groth16::Parameters<Bls12>,
        mut rng: R,
    ) -> Result<groth16::Proof<Bls12>, SynthesisError> {
        groth16::create_random_proof(multi_frame, params, &mut rng)
    }

    /// Generates an outer Groth16 proof using the given parameters, SRS, expression, environment,
    /// store, limit, and random number generator.
    pub fn outer_prove<R: RngCore + Clone>(
        &self,
        params: &groth16::Parameters<Bls12>,
        srs: &GenericSRS<Bls12>,
        expr: Ptr<Scalar>,
        env: Ptr<Scalar>,
        store: &mut Store<Scalar>,
        limit: usize,
        mut rng: R,
        lang: Arc<Lang<Scalar, C>>,
    ) -> Result<(Proof<Bls12>, IO<Scalar>, IO<Scalar>), ProofError> {
        let frames = self.get_evaluation_frames(expr, env, store, limit, lang.clone())?;
        let reduction_count = self.reduction_count();
        let folding_config = Arc::new(FoldingConfig::new_ivc(lang, reduction_count));
        let multiframes =
            MultiFrame::from_frames(reduction_count, &frames, store, folding_config.clone());
        let mut proofs = Vec::with_capacity(multiframes.len());
        let mut statements = Vec::with_capacity(multiframes.len());

        // NOTE: frame_proofs are not really needed, but having them helps with
        // testing and building confidence as we work up to fully succinct proofs.
        // Once these are removed a lot of the cloning and awkwardness of assembling
        // results here can be eliminated.
        let multiframes_count = multiframes.len();
        let mut multiframe_proofs = Vec::with_capacity(multiframes_count);

        let last_multiframe = multiframes.last().unwrap().clone();
        for multiframe in multiframes {
            statements.push(multiframe.public_inputs());
            let proof = self.prove(multiframe.clone(), params, &mut rng).unwrap();

            proofs.push(proof.clone());
            multiframe_proofs.push((multiframe, proof));
        }

        if proofs.len().count_ones() != 1 || proofs.len() < 2 {
            let dummy_multiframe = MultiFrame::make_dummy(
                self.reduction_count(),
                last_multiframe.frames.and_then(|x| x.last().cloned()),
                store,
                folding_config,
                Meta::Lurk,
            );

            let dummy_proof = self
                .prove(dummy_multiframe.clone(), params, &mut rng)
                .unwrap();

            let dummy_statement = dummy_multiframe.public_inputs();
            while proofs.len().count_ones() != 1 || proofs.len() < 2 {
                // Pad proofs and frames to a power of 2.
                proofs.push(dummy_proof.clone());
                statements.push(dummy_statement.clone());
            }
        }
        assert_eq!(1, statements.len().count_ones());

        let srs = srs.specialize_input_aggregation(proofs.len()).0;

        let proof = aggregate_proofs_and_instances(
            &srs,
            TRANSCRIPT_INCLUDE,
            statements.as_slice(),
            proofs.as_slice(),
            AggregateVersion::V2,
        )?;

        let public_inputs = frames[0].input;
        let public_outputs = frames[frames.len() - 1].output;

        Ok((
            Proof {
                proof,
                proof_count: proofs.len(),
                reduction_count: self.reduction_count(),
            },
            public_inputs,
            public_outputs,
        ))
    }

    /// Verifies a single Groth16 proof using the given multi_frame, prepared verifier key, and proof.
    pub fn verify_groth16_proof(
        // multiframe need not have inner frames populated for verification purposes.
        multiframe: &MultiFrame<'_, Scalar, C>,
        pvk: &groth16::PreparedVerifyingKey<Bls12>,
        proof: &groth16::Proof<Bls12>,
    ) -> Result<bool, SynthesisError> {
        let inputs = multiframe.public_inputs();

        verify_proof(pvk, proof, &inputs)
    }

    /// Verifies an aggregated Groth16 proof using the given prepared verifier key, SRS, public parameters, proof and rng.
    pub fn verify<R: RngCore + Send>(
        pvk: &groth16::PreparedVerifyingKey<Bls12>,
        srs_vk: &VerifierSRS<Bls12>,
        public_inputs: &[Scalar],
        public_outputs: &[Scalar],
        proof: &AggregateProofAndInstance<Bls12>,
        rng: &mut R,
    ) -> Result<bool, SynthesisError> {
        verify_aggregate_proof_and_aggregate_instances(
            srs_vk,
            pvk,
            rng,
            public_inputs,
            public_outputs,
            proof,
            TRANSCRIPT_INCLUDE,
            AggregateVersion::V2,
        )
    }
}

/// A prover struct for the Groth16 proving system.
/// Implements the crate::Prover trait.
#[derive(Debug)]
pub struct Groth16Prover<
    'a,
    E: MultiMillerLoop,
    C: Coprocessor<F> + 'a,
    F: LurkField,
    M: MultiFrameTrait<'a, F, C>,
> {
    reduction_count: usize,
    lang: Lang<F, C>,
    _p: PhantomData<(E, &'a M)>,
}

/// Public parameters for the Groth16 proving system.
/// implements the crate::PublicParameters trait.
pub struct PublicParams<E: Engine + MultiMillerLoop>(pub groth16::Parameters<E>);

impl PublicParameters for PublicParams<Bls12> {}

impl<'a, C: Coprocessor<Scalar>, M: MultiFrameTrait<'a, Scalar, C>> Prover<'a, Scalar, C, M>
    for Groth16Prover<'a, Bls12, C, Scalar, M>
{
    type PublicParams = PublicParams<Bls12>;

    fn new(reduction_count: usize, lang: Lang<Scalar, C>) -> Self {
        Groth16Prover {
            reduction_count,
            lang,
            _p: Default::default(),
        }
    }

    fn reduction_count(&self) -> usize {
        self.reduction_count
    }

    fn lang(&self) -> &Lang<Scalar, C> {
        &self.lang
    }
}

impl<C: Coprocessor<Scalar>> MultiFrame<'_, <Bls12 as Engine>::Fr, C> {
    /// Verify a Groth16 Lurk proof.
    pub fn verify_groth16_proof(
        self,
        pvk: &groth16::PreparedVerifyingKey<Bls12>,
        proof: &groth16::Proof<Bls12>,
    ) -> Result<bool, SynthesisError> {
        let inputs: Vec<Scalar> = self.public_inputs();
        verify_proof(pvk, proof, inputs.as_slice())
    }
}

#[allow(dead_code)]
fn verify_sequential_groth16_proofs<C: Coprocessor<Scalar>>(
    multiframe_proofs: &[(MultiFrame<'_, Scalar, C>, groth16::Proof<Bls12>)],
    vk: &groth16::VerifyingKey<Bls12>,
) -> Result<bool, SynthesisError> {
    let pvk = groth16::prepare_verifying_key(vk);

    for (i, (multiframe, proof)) in multiframe_proofs.iter().enumerate() {
        if i > 0 {
            let prev = &multiframe_proofs[i - 1].0;

            if !prev.precedes(multiframe) {
                return Ok(false);
            }
        }

        if !multiframe.clone().verify_groth16_proof(&pvk, proof)? {
            return Ok(false);
        }
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::ToInputs;
    use crate::eval::{empty_sym_env, lang::Coproc, Evaluator, Frame};
    use crate::lurk_sym_ptr;
    use crate::proof::{verify_sequential_css, SequentialCS};
    use bellpepper::util_cs::{metric_cs::MetricCS, Comparable};
    use bellpepper_core::{Circuit, Delta};
    use bellperson::groth16::aggregate::verify_aggregate_proof_and_aggregate_instances;

    use blstrs::Scalar as Fr;
    use rand::rngs::OsRng;

    const DEFAULT_CHECK_GROTH16: bool = false;
    const DEFAULT_REDUCTION_COUNT: usize = 5;

    fn outer_prove_aux<Fo: Fn(&'_ mut Store<Fr>) -> Ptr<Fr>>(
        source: &str,
        expected_result: Fo,
        expected_iterations: usize,
        check_groth16: bool,
        check_constraint_systems: bool,
        limit: usize,
        debug: bool,
    ) {
        let mut s = Store::default();
        let expected_result = expected_result(&mut s);

        let expr = s.read(source).unwrap();
        let lang = Lang::<Fr, Coproc<Fr>>::new();

        outer_prove_aux0(
            &mut s,
            expr,
            expected_result,
            expected_iterations,
            check_groth16,
            check_constraint_systems,
            limit,
            debug,
            &lang,
        )
    }

    fn outer_prove_aux0<C: Coprocessor<Fr>>(
        s: &mut Store<Fr>,
        expr: Ptr<Fr>,
        expected_result: Ptr<Fr>,
        expected_iterations: usize,
        check_groth16: bool,
        check_constraint_systems: bool,
        limit: usize,
        debug: bool,
        lang: &Lang<Fr, C>,
    ) {
        let rng = OsRng;

        let lang_rc = Arc::new(lang.clone());
        let public_params = Groth16Prover::<_, C, Fr, MultiFrame<'_, Fr, C>>::create_groth_params(
            DEFAULT_REDUCTION_COUNT,
            lang_rc.clone(),
        )
        .unwrap();
        let groth_prover = Groth16Prover::new(DEFAULT_REDUCTION_COUNT, lang.clone());
        let groth_params = &public_params.0;

        let pvk = groth16::prepare_verifying_key(&groth_params.vk);

        let e = empty_sym_env(s);

        if check_constraint_systems {
            let padding_predicate = |count| groth_prover.needs_frame_padding(count);
            let frames =
                Evaluator::generate_frames(expr, e, s, limit, padding_predicate, lang).unwrap();
            s.hydrate_scalar_cache();

            let folding_config = Arc::new(FoldingConfig::new_ivc(
                lang_rc.clone(),
                DEFAULT_REDUCTION_COUNT,
            ));

            let multi_frames =
                MultiFrame::from_frames(DEFAULT_REDUCTION_COUNT, &frames, s, folding_config);

            let cs = groth_prover.outer_synthesize(&multi_frames).unwrap();

            let _adjusted_iterations = groth_prover.expected_total_iterations(expected_iterations);

            if !debug {
                assert_eq!(expected_iterations, Frame::significant_frame_count(&frames));
                // This test fails sometimes because we are using outer_synthesize to get the frames.
                // That method only really exists to let us test synthesis without proofs, and it doesn't duplicate
                // all the padding logic required for SnarkPack. It might be nice to eventually refactor such taht it does,
                // in which case this check will be useful. So let's leave it around for now.
                // assert_eq!(adjusted_iterations, cs.len());
                assert!(s
                    .ptr_eq(&expected_result, &cs[cs.len() - 1].0.output.unwrap().expr)
                    .unwrap());
            }

            let constraint_systems_verified =
                verify_sequential_css::<Scalar, C, MultiFrame<'_, Fr, C>>(&cs).unwrap();
            assert!(constraint_systems_verified);

            check_cs_deltas::<C>(&cs, limit, lang_rc.clone());
        }

        let proof_results = (check_groth16).then(|| {
            groth_prover
                .outer_prove(
                    groth_params,
                    &INNER_PRODUCT_SRS,
                    expr,
                    empty_sym_env(s),
                    s,
                    limit,
                    rng,
                    lang_rc,
                )
                .unwrap()
        });

        if let Some((proof, public_inputs, public_outputs)) = proof_results {
            let srs_vk = INNER_PRODUCT_SRS.specialize_vk(proof.proof_count);
            let aggregate_proof_and_instances_verified =
                verify_aggregate_proof_and_aggregate_instances(
                    &srs_vk,
                    &pvk,
                    rng,
                    &public_inputs.to_inputs(s),
                    &public_outputs.to_inputs(s),
                    &proof.proof,
                    TRANSCRIPT_INCLUDE,
                    AggregateVersion::V2,
                )
                .unwrap();
            assert!(aggregate_proof_and_instances_verified);
        };
    }

    fn check_cs_deltas<C: Coprocessor<Fr>>(
        constraint_systems: &SequentialCS<Fr, MultiFrame<'_, Fr, C>>,
        limit: usize,
        lang: Arc<Lang<Fr, C>>,
    ) {
        let mut cs_blank = MetricCS::<Fr>::new();
        let folding_config = Arc::new(FoldingConfig::new_ivc(lang, DEFAULT_REDUCTION_COUNT));
        let blank_frame = MultiFrame::<'_, Scalar, C>::blank(folding_config, Meta::Lurk);
        blank_frame
            .synthesize(&mut cs_blank)
            .expect("failed to synthesize");

        for (_, (_frame, cs)) in constraint_systems.iter().take(limit).enumerate() {
            let delta = cs.delta(&cs_blank, true);
            assert!(delta == Delta::Equal);
        }
    }

    #[test]
    #[ignore]
    fn outer_prove_arithmetic_let() {
        outer_prove_aux(
            "(let ((a 5)
                      (b 1)
                      (c 2))
                 (/ (+ a b) c))",
            |store| store.num(3),
            18,
            DEFAULT_CHECK_GROTH16,
            true,
            128,
            false,
        );
    }

    #[test]
    #[ignore]
    fn outer_prove_binop() {
        outer_prove_aux(
            "(+ 1 2)",
            |store| store.num(3),
            3,
            DEFAULT_CHECK_GROTH16,
            true,
            128,
            false,
        );
    }

    #[test]
    #[ignore]
    fn outer_prove_eq() {
        outer_prove_aux(
            "(eq 5 5)",
            |store| lurk_sym_ptr!(store, t),
            3,
            true, // Always check Groth16 in at least one test.
            true,
            128,
            false,
        );
    }

    #[test]
    #[ignore]
    fn outer_prove_num_equal() {
        outer_prove_aux(
            "(= 5 5)",
            |store| lurk_sym_ptr!(store, t),
            3,
            DEFAULT_CHECK_GROTH16,
            true,
            128,
            false,
        );
        outer_prove_aux(
            "(= 5 6)",
            |store| lurk_sym_ptr!(store, nil),
            3,
            DEFAULT_CHECK_GROTH16,
            true,
            128,
            false,
        );
    }

    #[test]
    #[ignore]
    fn outer_prove_if() {
        outer_prove_aux(
            "(if t 5 6)",
            |store| store.num(5),
            3,
            DEFAULT_CHECK_GROTH16,
            true,
            128,
            false,
        );

        outer_prove_aux(
            "(if t 5 6)",
            |store| store.num(5),
            3,
            DEFAULT_CHECK_GROTH16,
            true,
            128,
            false,
        )
    }
    #[test]
    #[ignore]
    fn outer_prove_if_fully_evaluates() {
        outer_prove_aux(
            "(if t (+ 5 5) 6)",
            |store| store.num(10),
            5,
            DEFAULT_CHECK_GROTH16,
            true,
            128,
            false,
        );
    }

    #[test]
    #[ignore]
    fn outer_prove_recursion1() {
        outer_prove_aux(
            "(letrec ((exp (lambda (base)
                                (lambda (exponent)
                                  (if (= 0 exponent)
                                      1
                                      (* base ((exp base) (- exponent 1))))))))
                 ((exp 5) 3))",
            |store| store.num(125),
            // 117, // FIXME: is this change correct?
            91,
            DEFAULT_CHECK_GROTH16,
            true,
            256,
            false,
        );
    }

    #[test]
    #[ignore]
    fn outer_prove_recursion2() {
        outer_prove_aux(
            "(letrec ((exp (lambda (base)
                                   (lambda (exponent)
                                      (lambda (acc)
                                        (if (= 0 exponent)
                                           acc
                                           (((exp base) (- exponent 1)) (* acc base))))))))
                (((exp 5) 5) 1))",
            |store| store.num(3125),
            // 248, // FIXME: is this change correct?
            201,
            DEFAULT_CHECK_GROTH16,
            true,
            256,
            false,
        );
    }

    #[test]
    #[ignore]
    fn outer_prove_chained_functional_commitment() {
        let mut s = Store::<Fr>::default();

        let fun_src = s
            .read(
                "(letrec ((secret 12345)
                          (a (lambda (acc x)
                               (let ((acc (+ acc x)))
                                 (cons acc (cons secret (a acc)))))))
                   (a 0))",
            )
            .unwrap();
        let limit = 300;
        let lang = Lang::<Fr, Coproc<Fr>>::new();

        let (evaled, _, _) = Evaluator::new(fun_src, empty_sym_env(&s), &s, limit, &lang)
            .eval()
            .unwrap();

        let fun = evaled.expr;

        let cdr = lurk_sym_ptr!(s, cdr);
        let quote = lurk_sym_ptr!(s, quote);

        let zero = s.num(0);
        let five = s.num(5);
        let commitment = s.cons(zero, fun);
        let quoted_commitment = s.list(&[quote, commitment]);
        let fun_from_comm = s.list(&[cdr, quoted_commitment]);
        let input = s.list(&[fun_from_comm, five]);

        let (output, _iterations, _emitted) =
            Evaluator::new(input, empty_sym_env(&s), &s, limit, &lang)
                .eval()
                .unwrap();

        let result_expr = output.expr;

        outer_prove_aux0(
            &mut s,
            input,
            result_expr,
            32,
            true,
            true,
            limit,
            false,
            &lang,
        );
    }
}
