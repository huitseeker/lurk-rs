use std::env::current_dir;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use lurk::circuit::gadgets::circom::multiply::circom_multiply;
use lurk::circuit::gadgets::data::GlobalAllocations;
use lurk::circuit::gadgets::pointer::{AllocatedContPtr, AllocatedPtr};
use lurk::coprocessor::{CoCircuit, Coprocessor};
use lurk::eval::{empty_sym_env, lang::Lang};
use lurk::field::LurkField;
use lurk::proof::{nova::NovaProver, Prover};
use lurk::ptr::Ptr;
use lurk::public_parameters::public_params;
use lurk::store::Store;
use lurk::{Num, Symbol};
use lurk_macros::Coproc;

use bellperson::{ConstraintSystem, SynthesisError};

use nova_scotia::r1cs::CircomConfig;
use pasta_curves::pallas::Scalar as Fr;

const REDUCTION_COUNT: usize = 1;

#[derive(Debug)]
pub(crate) struct CircomMultiplyCoprocessor<F: LurkField> {
    root: PathBuf,
    circom_config: CircomConfig<F>,
}

impl<F: LurkField> Clone for CircomMultiplyCoprocessor<F> {
    fn clone(&self) -> Self {
        CircomMultiplyCoprocessor::new(self.root.clone())
    }
}

impl<F: LurkField> CoCircuit<F> for CircomMultiplyCoprocessor<F> {
    fn arity(&self) -> usize {
        0
    }

    fn synthesize<CS: ConstraintSystem<F>>(
        &self,
        cs: &mut CS,
        g: &GlobalAllocations<F>,
        store: &Store<F>,
        _input_exprs: &[AllocatedPtr<F>],
        input_env: &AllocatedPtr<F>,
        input_cont: &AllocatedContPtr<F>,
    ) -> Result<(AllocatedPtr<F>, AllocatedPtr<F>, AllocatedContPtr<F>), SynthesisError> {

        let output = circom_multiply(
            &mut cs.namespace(|| "circom_multiply"),
            F::from(5),
            F::from(7),
            &self.circom_config,
        )?;


        let res = AllocatedPtr::from_parts(g.num_tag.clone(), output);

        Ok((res, input_env.clone(), input_cont.clone()))
    }
}

impl<F: LurkField> Coprocessor<F> for CircomMultiplyCoprocessor<F> {
    fn eval_arity(&self) -> usize {
        0
    }

    fn simple_evaluate(&self, s: &mut Store<F>, _args: &[Ptr<F>]) -> Ptr<F> {
        let expected = Num::Scalar(F::from_str_vartime("35").unwrap());
        s.intern_num(expected)
    }

    fn has_circuit(&self) -> bool {
        true
    }
}

impl<F: LurkField> CircomMultiplyCoprocessor<F> {
    pub(crate) fn new(root: PathBuf) -> Self {
        let mut wtns = root.clone();
        wtns.push("main_js");
        wtns.push("main");
        wtns.set_extension("wasm");
        let mut r1cs = root.clone();
        r1cs.push("main");
        r1cs.set_extension("r1cs");
        Self {
            root,
            circom_config: CircomConfig::new(wtns, r1cs).unwrap(),
        }
    }
}

#[derive(Clone, Debug, Coproc)]
enum MultiplyCoproc<F: LurkField> {
    SC(CircomMultiplyCoprocessor<F>),
}

/// Run the example in this file with
/// `cargo run --release --example circom_sha256`
fn main() {        
    let mut root = current_dir().unwrap();
    root.push("src/circuit/gadgets/circom/multiply");
    let mut wtns = root.clone();
    wtns.push("main_js/main.wasm");
    let mut r1cs = root.clone();
    r1cs.push("main.r1cs");


    let store = &mut Store::<Fr>::new();
    let sym_str = Symbol::new(&[".circom_multiply"]); // two inputs
    let lang = Lang::<Fr, MultiplyCoproc<Fr>>::new_with_bindings(
        store,
        vec![(sym_str.clone(), CircomMultiplyCoprocessor::new(root).into())],
    );

    let coproc_expr = format!("{}", sym_str);
    dbg!(coproc_expr.clone());

    let expr = format!("({coproc_expr})");
    let ptr = store.read(&expr).unwrap();

    let nova_prover = NovaProver::<Fr, MultiplyCoproc<Fr>>::new(REDUCTION_COUNT, lang.clone());
    let lang_rc = Arc::new(lang);

    println!("Setting up public parameters...");

    let pp_start = Instant::now();
    let pp = public_params::<MultiplyCoproc<Fr>>(REDUCTION_COUNT, lang_rc.clone()).unwrap();
    let pp_end = pp_start.elapsed();

    println!("Public parameters took {:?}", pp_end);

    println!("Beginning proof step...");

    let proof_start = Instant::now();
    let (proof, z0, zi, num_steps) = nova_prover
        .evaluate_and_prove(&pp, ptr, empty_sym_env(store), store, 10000, lang_rc)
        .unwrap();
    let proof_end = proof_start.elapsed();

    println!("Proofs took {:?}", proof_end);

    println!("Verifying proof...");

    let verify_start = Instant::now();
    let res = proof.verify(&pp, num_steps, &z0, &zi).unwrap();
    let verify_end = verify_start.elapsed();

    println!("Verify took {:?}", verify_end);

    if res {
        println!(
            "Congratulations! You proved and verified a CIRCOM-SHA256 hash calculation in {:?} time!",
            pp_end + proof_end + verify_end
        );
    }
}